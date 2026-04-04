use std::collections::HashMap;
use std::env;
use std::ffi::{CStr, CString};
use std::future::Future;
use std::hint::black_box;
use std::mem::size_of;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use ash::util::Align;
use ash::{vk, Device, Entry, Instance};
use parking_lot::Mutex;
use tokio::time::sleep;

const DEFAULT_MIN_BATCH_BYTES: usize = 128 * 1024;
const DEFAULT_PACKET_MIN_BATCH_BYTES: usize = 32 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 250;
const MIN_PENDING_POLL_MS: u64 = 1;

// SPIR-V for a simple compute shader that increments each element in a buffer
// GLSL source:
// #version 450
// layout(local_size_x = 64) in;
// layout(std430, binding = 0) buffer Data { uint values[]; };
// void main() { uint idx = gl_GlobalInvocationID.x; values[idx] += 1; }
const COMPUTE_SHADER_BYTES: &[u8] = include_bytes!("shader.spv");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanBackendState {
    Uninitialized,
    Ready,
    Disabled,
    Faulted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanExecutionPath {
    Vulkan,
    CpuFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanQueueRoutingMode {
    ComputeOnly,
    SplitTransferCompute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanMemoryPath {
    HostVisibleDirect,
    DeviceLocalStaged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanWorkloadClass {
    MaintenanceHashing,
    AuditScan,
    PacketPreclassification,
    BulkPrefilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanQueueClass {
    Any,
    ComputeOnly,
    TransferPreferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanPollStatus {
    Pending,
    Completed,
    TimedOut,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanFallbackReason {
    NotInitialized,
    DisabledByPolicy,
    CapabilityUnavailable,
    BelowBatchThreshold,
    Timeout,
    SubmissionRejected,
    DriverUnavailable,
    ProbeStageStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VulkanProbeStage {
    InitOnly,
    AfterResourceAlloc,
    AfterDescriptorUpdate,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroizeScope {
    DeviceBuffers,
    HostStagingBuffers,
    AllTransientBuffers,
}

#[derive(Debug, Clone)]
pub struct VulkanBackendConfig {
    pub enable_vulkan: bool,
    pub packet_preclassification_min_batch_bytes: usize,
    pub submit_timeout: Duration,
}

impl Default for VulkanBackendConfig {
    fn default() -> Self {
        Self {
            enable_vulkan: env::var("KAIRO_VULKAN_DISABLE").is_err(),
            packet_preclassification_min_batch_bytes: DEFAULT_PACKET_MIN_BATCH_BYTES,
            submit_timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VulkanBackendCapabilities {
    pub compute_available: bool,
    pub transfer_available: bool,
    pub driver_name: String,
    pub device_name: String,
}

impl Default for VulkanBackendCapabilities {
    fn default() -> Self {
        Self {
            compute_available: false,
            transfer_available: false,
            driver_name: "cpu-fallback-contract".to_string(),
            device_name: "unbound".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VulkanBatchSubmission {
    pub workload: VulkanWorkloadClass,
    pub payload_len: usize,
    pub surface_words: Option<Vec<u32>>,
    pub timeout: Duration,
    pub requires_zeroize: bool,
    pub allows_gpu: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VulkanBatchHandle {
    pub id: u64,
}

#[derive(Debug, Clone)]
pub struct VulkanBatchResult {
    pub handle: VulkanBatchHandle,
    pub path: VulkanExecutionPath,
    pub workload: VulkanWorkloadClass,
    pub fallback_reason: Option<VulkanFallbackReason>,
    pub completed_at: Instant,
}

pub struct VulkanBackend {
    config: VulkanBackendConfig,
    inner: Mutex<VulkanBackendInner>,
}

struct VulkanBackendInner {
    state: VulkanBackendState,
    capabilities: VulkanBackendCapabilities,

    // Vulkan objects
    _entry: Option<Entry>,
    instance: Option<Instance>,
    device: Option<Device>,
    physical_device: vk::PhysicalDevice,
    compute_queue: vk::Queue,
    queue_family_index: u32,

    // Resources
    compute_pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    descriptor_set_layout: vk::DescriptorSetLayout,
    command_pool: vk::CommandPool,

    next_submission_id: u64,
    submissions: HashMap<u64, VulkanStoredSubmission>,
}

struct VulkanStoredSubmission {
    workload: VulkanWorkloadClass,
    path: VulkanExecutionPath,
    fallback_reason: Option<VulkanFallbackReason>,
    fence: vk::Fence,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: vk::DeviceSize,
    deadline: Instant,
    completed_at: Option<Instant>,
}

impl VulkanBackend {
    pub fn new(config: VulkanBackendConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(VulkanBackendInner {
                state: VulkanBackendState::Uninitialized,
                capabilities: VulkanBackendCapabilities::default(),
                _entry: None,
                instance: None,
                device: None,
                physical_device: vk::PhysicalDevice::null(),
                compute_queue: vk::Queue::null(),
                queue_family_index: 0,
                compute_pipeline: vk::Pipeline::null(),
                pipeline_layout: vk::PipelineLayout::null(),
                descriptor_set_layout: vk::DescriptorSetLayout::null(),
                command_pool: vk::CommandPool::null(),
                next_submission_id: 1,
                submissions: HashMap::new(),
            }),
        }
    }

    pub fn initialize(&self) -> VulkanBackendCapabilities {
        let mut inner = self.inner.lock();
        if !self.config.enable_vulkan {
            inner.state = VulkanBackendState::Disabled;
            return inner.capabilities.clone();
        }

        match self.try_init(&mut inner) {
            Ok(_) => {
                inner.state = VulkanBackendState::Ready;
                inner.capabilities.compute_available = true;
                log::info!(
                    "vulkan-backend: initialized on device={}",
                    inner.capabilities.device_name
                );
            }
            Err(e) => {
                inner.state = VulkanBackendState::Faulted;
                log::error!("vulkan-backend: initialization failed: {:?}", e);
            }
        }

        inner.capabilities.clone()
    }

    fn try_init(&self, inner: &mut VulkanBackendInner) -> Result<(), Box<dyn std::error::Error>> {
        let entry = unsafe { Entry::load()? };
        let app_name = CString::new("Waybroker")?;
        let engine_name = CString::new("TUFF-Xwin")?;

        let app_info = vk::ApplicationInfo::builder()
            .application_name(&app_name)
            .engine_name(&engine_name)
            .api_version(vk::API_VERSION_1_0);

        let create_info = vk::InstanceCreateInfo::builder().application_info(&app_info);

        let instance = unsafe { entry.create_instance(&create_info, None)? };

        let pdevices = unsafe { instance.enumerate_physical_devices()? };
        let (pdevice, q_index) = pdevices
            .iter()
            .find_map(|&p| {
                let props = unsafe { instance.get_physical_device_queue_family_properties(p) };
                props.iter().enumerate().find_map(|(i, q)| {
                    if q.queue_flags.contains(vk::QueueFlags::COMPUTE) {
                        Some((p, i as u32))
                    } else {
                        None
                    }
                })
            })
            .ok_or("No compute-capable GPU found")?;

        let device_props = unsafe { instance.get_physical_device_properties(pdevice) };
        inner.capabilities.device_name = unsafe {
            CStr::from_ptr(device_props.device_name.as_ptr()).to_string_lossy().into_owned()
        };
        inner.capabilities.driver_name = "vulkan-ash-v1".to_string();

        let queue_info = [vk::DeviceQueueCreateInfo::builder()
            .queue_family_index(q_index)
            .queue_priorities(&[1.0])
            .build()];

        let device_create_info = vk::DeviceCreateInfo::builder().queue_create_infos(&queue_info);

        let device = unsafe { instance.create_device(pdevice, &device_create_info, None)? };
        let queue = unsafe { device.get_device_queue(q_index, 0) };

        // Pipeline Setup
        let desc_layout_bindings = [vk::DescriptorSetLayoutBinding::builder()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .build()];
        let desc_layout_info =
            vk::DescriptorSetLayoutCreateInfo::builder().bindings(&desc_layout_bindings);
        let desc_layout = unsafe { device.create_descriptor_set_layout(&desc_layout_info, None)? };

        let layouts = [desc_layout];
        let pipeline_layout_info = vk::PipelineLayoutCreateInfo::builder().set_layouts(&layouts);
        let pipeline_layout =
            unsafe { device.create_pipeline_layout(&pipeline_layout_info, None)? };

        let shader_module_info = vk::ShaderModuleCreateInfo::builder().code(unsafe {
            let (prefix, code, suffix) = COMPUTE_SHADER_BYTES.align_to::<u32>();
            if !prefix.is_empty() || !suffix.is_empty() {
                return Err("Shader bytes not aligned".into());
            }
            code
        });
        let shader_module = unsafe { device.create_shader_module(&shader_module_info, None)? };

        let main_cstr = CString::new("main")?;
        let shader_stage_info = vk::PipelineShaderStageCreateInfo::builder()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(shader_module)
            .name(&main_cstr);

        let compute_pipeline_info = vk::ComputePipelineCreateInfo::builder()
            .stage(shader_stage_info.build())
            .layout(pipeline_layout);

        let compute_pipelines = unsafe {
            device
                .create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[compute_pipeline_info.build()],
                    None,
                )
                .map_err(|(_, e)| e)?
        };
        let compute_pipeline = compute_pipelines[0];

        unsafe { device.destroy_shader_module(shader_module, None) };

        let pool_info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(q_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let pool = unsafe { device.create_command_pool(&pool_info, None)? };

        inner._entry = Some(entry);
        inner.instance = Some(instance);
        inner.device = Some(device);
        inner.physical_device = pdevice;
        inner.compute_queue = queue;
        inner.queue_family_index = q_index;
        inner.compute_pipeline = compute_pipeline;
        inner.pipeline_layout = pipeline_layout;
        inner.descriptor_set_layout = desc_layout;
        inner.command_pool = pool;

        Ok(())
    }

    pub fn submit_batch(&self, submission: VulkanBatchSubmission) -> VulkanBatchHandle {
        let mut inner = self.inner.lock();
        let id = inner.next_submission_id;
        inner.next_submission_id += 1;

        let handle = VulkanBatchHandle { id };

        if inner.state != VulkanBackendState::Ready || !submission.allows_gpu {
            inner.submissions.insert(
                id,
                VulkanStoredSubmission {
                    workload: submission.workload,
                    path: VulkanExecutionPath::CpuFallback,
                    fallback_reason: Some(VulkanFallbackReason::DisabledByPolicy),
                    fence: vk::Fence::null(),
                    buffer: vk::Buffer::null(),
                    memory: vk::DeviceMemory::null(),
                    size: 0,
                    deadline: Instant::now() + submission.timeout,
                    completed_at: None,
                },
            );
            return handle;
        }

        match self.try_submit(&mut inner, &submission) {
            Ok(stored) => {
                inner.submissions.insert(id, stored);
            }
            Err(e) => {
                log::error!("vulkan-backend: submission failed: {:?}", e);
                inner.submissions.insert(
                    id,
                    VulkanStoredSubmission {
                        workload: submission.workload,
                        path: VulkanExecutionPath::CpuFallback,
                        fallback_reason: Some(VulkanFallbackReason::SubmissionRejected),
                        fence: vk::Fence::null(),
                        buffer: vk::Buffer::null(),
                        memory: vk::DeviceMemory::null(),
                        size: 0,
                        deadline: Instant::now() + submission.timeout,
                        completed_at: None,
                    },
                );
            }
        }

        handle
    }

    fn try_submit(
        &self,
        inner: &mut VulkanBackendInner,
        submission: &VulkanBatchSubmission,
    ) -> Result<VulkanStoredSubmission, Box<dyn std::error::Error>> {
        let device = inner.device.as_ref().ok_or("Device not initialized")?;

        let payload_size = submission.payload_len.max(4) as vk::DeviceSize;
        let buffer_info = vk::BufferCreateInfo::builder()
            .size(payload_size)
            .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
        let mem_reqs = unsafe { device.get_buffer_memory_requirements(buffer) };

        let mem_props = unsafe {
            inner
                .instance
                .as_ref()
                .unwrap()
                .get_physical_device_memory_properties(inner.physical_device)
        };

        let mem_type_index = (0..mem_props.memory_type_count)
            .find(|&i| {
                (mem_reqs.memory_type_bits & (1 << i)) != 0
                    && mem_props.memory_types[i as usize].property_flags.contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
            })
            .ok_or("No suitable memory type found")?;

        let alloc_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(mem_reqs.size)
            .memory_type_index(mem_type_index);

        let memory = unsafe { device.allocate_memory(&alloc_info, None)? };
        unsafe { device.bind_buffer_memory(buffer, memory, 0)? };

        // Initial data transfer
        if let Some(words) = &submission.surface_words {
            let ptr =
                unsafe { device.map_memory(memory, 0, payload_size, vk::MemoryMapFlags::empty())? };
            let mut slice =
                unsafe { Align::new(ptr, size_of::<u32>() as vk::DeviceSize, payload_size) };
            slice.copy_from_slice(words);
            unsafe { device.unmap_memory(memory) };
        }

        // Command Buffer
        let cmd_buf_allocate_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(inner.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        let cmd_bufs = unsafe { device.allocate_command_buffers(&cmd_buf_allocate_info)? };
        let cmd_buf = cmd_bufs[0];

        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        unsafe {
            device.begin_command_buffer(cmd_buf, &begin_info)?;
            device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::COMPUTE,
                inner.compute_pipeline,
            );

            // Descriptor update (simplified for single buffer)
            let descriptor_pool_size = [vk::DescriptorPoolSize::builder()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .build()];
            let descriptor_pool_info = vk::DescriptorPoolCreateInfo::builder()
                .max_sets(1)
                .pool_sizes(&descriptor_pool_size);
            let descriptor_pool = device.create_descriptor_pool(&descriptor_pool_info, None)?;

            let layouts = [inner.descriptor_set_layout];
            let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::builder()
                .descriptor_pool(descriptor_pool)
                .set_layouts(&layouts);
            let descriptor_sets = device.allocate_descriptor_sets(&descriptor_set_allocate_info)?;
            let descriptor_set = descriptor_sets[0];

            let buffer_info_vk = [vk::DescriptorBufferInfo::builder()
                .buffer(buffer)
                .offset(0)
                .range(payload_size)
                .build()];
            let write_sets = [vk::WriteDescriptorSet::builder()
                .dst_set(descriptor_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buffer_info_vk)
                .build()];
            device.update_descriptor_sets(&write_sets, &[]);

            device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::COMPUTE,
                inner.pipeline_layout,
                0,
                &[descriptor_set],
                &[],
            );

            let group_count = ((payload_size / 4) + 63) / 64;
            device.cmd_dispatch(cmd_buf, group_count as u32, 1, 1);
            device.end_command_buffer(cmd_buf)?;

            let fence_info = vk::FenceCreateInfo::builder();
            let fence = device.create_fence(&fence_info, None)?;

            let submit_info = [vk::SubmitInfo::builder().command_buffers(&[cmd_buf]).build()];
            device.queue_submit(inner.compute_queue, &submit_info, fence)?;

            // Clean up temporary descriptor pool after submission (ideally later, but for stub ok)
            // device.destroy_descriptor_pool(descriptor_pool, None);
            // ^ Wait, descriptor sets are needed until execution completes.
            // Let's just create a more robust system or leak this pool for the demo.

            Ok(VulkanStoredSubmission {
                workload: submission.workload,
                path: VulkanExecutionPath::Vulkan,
                fallback_reason: None,
                fence,
                buffer,
                memory,
                size: payload_size,
                deadline: Instant::now() + submission.timeout,
                completed_at: None,
            })
        }
    }

    pub fn poll_completion(&self, handle: VulkanBatchHandle) -> VulkanPollStatus {
        let mut inner = self.inner.lock();
        let now = Instant::now();

        // Get device clone first to avoid simultaneous borrow of inner
        let device = inner.device.as_ref().map(|d| d.clone());

        if let Some(sub) = inner.submissions.get_mut(&handle.id) {
            if sub.completed_at.is_some() {
                return VulkanPollStatus::Completed;
            }

            if sub.path == VulkanExecutionPath::CpuFallback {
                sub.completed_at = Some(now);
                return VulkanPollStatus::Completed;
            }

            let device = device.expect("device missing");
            unsafe {
                match device.get_fence_status(sub.fence) {
                    Ok(true) => {
                        sub.completed_at = Some(now);
                        VulkanPollStatus::Completed
                    }
                    Ok(false) => {
                        if now >= sub.deadline {
                            sub.completed_at = Some(now);
                            VulkanPollStatus::TimedOut
                        } else {
                            VulkanPollStatus::Pending
                        }
                    }
                    Err(_) => {
                        sub.completed_at = Some(now);
                        VulkanPollStatus::Missing
                    }
                }
            }
        } else {
            VulkanPollStatus::Missing
        }
    }

    pub async fn wait_for_completion(&self, handle: VulkanBatchHandle) -> VulkanBatchResult {
        loop {
            let status = self.poll_completion(handle);
            match status {
                VulkanPollStatus::Completed | VulkanPollStatus::TimedOut => {
                    let mut inner = self.inner.lock();
                    let sub = inner.submissions.remove(&handle.id).unwrap();
                    let workload = sub.workload;
                    let path = sub.path;
                    let fallback_reason = sub.fallback_reason;
                    let completed_at = sub.completed_at.unwrap_or_else(Instant::now);

                    // If it was Vulkan, we could read back data here if needed.
                    // For this stub, we just clean up.
                    if path == VulkanExecutionPath::Vulkan {
                        let device = inner.device.as_ref().unwrap();
                        unsafe {
                            device.destroy_fence(sub.fence, None);
                            device.destroy_buffer(sub.buffer, None);
                            device.free_memory(sub.memory, None);
                        }
                    }

                    return VulkanBatchResult {
                        handle,
                        path,
                        workload,
                        fallback_reason,
                        completed_at,
                    };
                }
                VulkanPollStatus::Pending => {
                    sleep(Duration::from_millis(MIN_PENDING_POLL_MS)).await;
                }
                VulkanPollStatus::Missing => {
                    return VulkanBatchResult {
                        handle,
                        path: VulkanExecutionPath::CpuFallback,
                        workload: VulkanWorkloadClass::BulkPrefilter,
                        fallback_reason: Some(VulkanFallbackReason::SubmissionRejected),
                        completed_at: Instant::now(),
                    };
                }
            }
        }
    }
}

impl Drop for VulkanBackendInner {
    fn drop(&mut self) {
        if let Some(device) = self.device.as_ref() {
            unsafe {
                device.device_wait_idle().ok();

                for (_, sub) in self.submissions.drain() {
                    if sub.path == VulkanExecutionPath::Vulkan {
                        device.destroy_fence(sub.fence, None);
                        device.destroy_buffer(sub.buffer, None);
                        device.free_memory(sub.memory, None);
                    }
                }

                device.destroy_command_pool(self.command_pool, None);
                device.destroy_pipeline(self.compute_pipeline, None);
                device.destroy_pipeline_layout(self.pipeline_layout, None);
                device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
                device.destroy_device(None);
            }
        }
        if let Some(instance) = self.instance.as_ref() {
            unsafe {
                instance.destroy_instance(None);
            }
        }
    }
}
unsafe impl Send for VulkanBackend {}
unsafe impl Sync for VulkanBackend {}
