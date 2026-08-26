use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, CString};
use std::io::Cursor;
use std::time::{Duration, Instant};

use ash::{vk, Device, Entry, Instance};
use parking_lot::Mutex;
use tokio::time::sleep;
use waybroker_common::accel::global_accel_policy;
use zeroize::Zeroize;

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

const DEFAULT_PACKET_MIN_BATCH_BYTES: usize = 32 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 250;
const MAX_TIMEOUT_MS: u64 = 5_000;
const MIN_PENDING_POLL_MS: u64 = 1;
const MAX_PENDING_GPU_SUBMISSIONS: usize = 64;
const MAX_TERMINAL_RESULTS: usize = MAX_PENDING_GPU_SUBMISSIONS * 2;
const MAX_GPU_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_SURFACE_WORDS: usize = 1_048_576;
const MAX_TOTAL_GPU_ALLOCATION_BYTES: u64 = 256 * 1024 * 1024;

// SPIR-V for the matching shader.comp probe/assist shader. The shader increments
// each u32 in the supplied metadata buffer. Caller-visible rendering and filtering
// semantics remain CPU/SIMD authoritative; this backend does not claim to be a
// pixel compositor or screenshot transform implementation.
const COMPUTE_SHADER_BYTES: &[u8] = include_bytes!("shader.spv");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VulkanBackendState {
    Uninitialized,
    Ready,
    Disabled,
    Faulted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VulkanWorkloadClass {
    MaintenanceHashing,
    AuditScan,
    PacketPreclassification,
    BulkPrefilter,
    ScreenshotRefine,
    SceneComposition,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        let accel_policy = global_accel_policy();
        Self {
            enable_vulkan: cfg!(feature = "accel-vulkan") && accel_policy.prefers_vulkan(),
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
    generation: u64,
    workload: VulkanWorkloadClass,
    path: VulkanExecutionPath,
    fallback_reason: Option<VulkanFallbackReason>,
}

#[derive(Debug, Clone)]
pub struct VulkanBatchResult {
    pub handle: VulkanBatchHandle,
    pub path: VulkanExecutionPath,
    pub workload: VulkanWorkloadClass,
    pub fallback_reason: Option<VulkanFallbackReason>,
    pub completed_at: Instant,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VulkanBackendMetrics {
    pub total_submissions: u64,
    pub vulkan_submissions: u64,
    pub cpu_fallback_submissions: u64,
    pub completed_vulkan_submissions: u64,
    pub timed_out_submissions: u64,
    pub driver_faults: u64,
    pub current_gpu_allocation_bytes: u64,
    pub peak_gpu_allocation_bytes: u64,
    pub zeroized_host_bytes: u64,
    pub zeroized_device_bytes: u64,
    pub pending_gpu_submissions: usize,
    pub quarantined_gpu_submissions: usize,
}

pub struct VulkanBackend {
    config: VulkanBackendConfig,
    inner: Mutex<VulkanBackendInner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SubmissionKey {
    generation: u64,
    id: u64,
}

struct VulkanBackendInner {
    state: VulkanBackendState,
    capabilities: VulkanBackendCapabilities,

    _entry: Option<Entry>,
    instance: Option<Instance>,
    device: Option<Device>,
    physical_device: vk::PhysicalDevice,
    compute_queue: vk::Queue,

    compute_pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    descriptor_set_layout: vk::DescriptorSetLayout,
    command_pool: vk::CommandPool,

    next_submission_id: u64,
    submission_generation: u64,
    handle_space_exhausted: bool,
    submissions: HashMap<SubmissionKey, VulkanStoredSubmission>,
    quarantined: HashMap<SubmissionKey, VulkanStoredSubmission>,
    terminal_results: HashMap<SubmissionKey, VulkanTerminalRecord>,
    terminal_order: VecDeque<SubmissionKey>,
    metrics: VulkanBackendMetrics,
}

struct VulkanStoredSubmission {
    workload: VulkanWorkloadClass,
    fence: vk::Fence,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    descriptor_pool: vk::DescriptorPool,
    command_buffer: vk::CommandBuffer,
    allocation_size: vk::DeviceSize,
    deadline: Instant,
    completed_at: Option<Instant>,
    requires_zeroize: bool,
}

#[derive(Debug, Clone, Copy)]
struct VulkanTerminalRecord {
    status: VulkanPollStatus,
    fallback_reason: VulkanFallbackReason,
    completed_at: Instant,
}

#[derive(Debug)]
enum SubmitError {
    Vulkan(vk::Result),
    Contract(&'static str),
}

impl From<vk::Result> for SubmitError {
    fn from(value: vk::Result) -> Self {
        Self::Vulkan(value)
    }
}

impl SubmitError {
    fn is_device_lost(&self) -> bool {
        matches!(self, Self::Vulkan(vk::Result::ERROR_DEVICE_LOST))
    }
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vulkan(error) => write!(formatter, "Vulkan error: {error:?}"),
            Self::Contract(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for SubmitError {}

struct VulkanInitGuard {
    entry: Option<Entry>,
    instance: Option<Instance>,
    device: Option<Device>,
    descriptor_set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    shader_module: vk::ShaderModule,
    compute_pipeline: vk::Pipeline,
    command_pool: vk::CommandPool,
}

impl VulkanInitGuard {
    fn new(entry: Entry) -> Self {
        Self {
            entry: Some(entry),
            instance: None,
            device: None,
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            shader_module: vk::ShaderModule::null(),
            compute_pipeline: vk::Pipeline::null(),
            command_pool: vk::CommandPool::null(),
        }
    }

    fn commit(
        mut self,
        inner: &mut VulkanBackendInner,
        physical_device: vk::PhysicalDevice,
        compute_queue: vk::Queue,
    ) {
        inner._entry = self.entry.take();
        inner.instance = self.instance.take();
        inner.device = self.device.take();
        inner.physical_device = physical_device;
        inner.compute_queue = compute_queue;
        inner.compute_pipeline =
            std::mem::replace(&mut self.compute_pipeline, vk::Pipeline::null());
        inner.pipeline_layout =
            std::mem::replace(&mut self.pipeline_layout, vk::PipelineLayout::null());
        inner.descriptor_set_layout =
            std::mem::replace(&mut self.descriptor_set_layout, vk::DescriptorSetLayout::null());
        inner.command_pool = std::mem::replace(&mut self.command_pool, vk::CommandPool::null());
    }
}

impl Drop for VulkanInitGuard {
    fn drop(&mut self) {
        if let Some(device) = self.device.as_ref() {
            unsafe {
                if self.command_pool != vk::CommandPool::null() {
                    device.destroy_command_pool(self.command_pool, None);
                }
                if self.compute_pipeline != vk::Pipeline::null() {
                    device.destroy_pipeline(self.compute_pipeline, None);
                }
                if self.shader_module != vk::ShaderModule::null() {
                    device.destroy_shader_module(self.shader_module, None);
                }
                if self.pipeline_layout != vk::PipelineLayout::null() {
                    device.destroy_pipeline_layout(self.pipeline_layout, None);
                }
                if self.descriptor_set_layout != vk::DescriptorSetLayout::null() {
                    device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
                }
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

struct PendingGpuResources<'a> {
    device: &'a Device,
    command_pool: vk::CommandPool,
    fence: vk::Fence,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    descriptor_pool: vk::DescriptorPool,
    command_buffer: vk::CommandBuffer,
    allocation_size: vk::DeviceSize,
    requires_zeroize: bool,
    armed: bool,
}

impl<'a> PendingGpuResources<'a> {
    fn new(device: &'a Device, command_pool: vk::CommandPool, requires_zeroize: bool) -> Self {
        Self {
            device,
            command_pool,
            fence: vk::Fence::null(),
            buffer: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            command_buffer: vk::CommandBuffer::null(),
            allocation_size: 0,
            requires_zeroize,
            armed: true,
        }
    }

    fn into_stored(
        mut self,
        workload: VulkanWorkloadClass,
        deadline: Instant,
    ) -> VulkanStoredSubmission {
        let stored = VulkanStoredSubmission {
            workload,
            fence: self.fence,
            buffer: self.buffer,
            memory: self.memory,
            descriptor_pool: self.descriptor_pool,
            command_buffer: self.command_buffer,
            allocation_size: self.allocation_size,
            deadline,
            completed_at: None,
            requires_zeroize: self.requires_zeroize,
        };
        self.armed = false;
        stored
    }
}

impl Drop for PendingGpuResources<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        unsafe {
            if self.requires_zeroize
                && self.memory != vk::DeviceMemory::null()
                && self.allocation_size != 0
            {
                let _ = zeroize_device_memory(self.device, self.memory, self.allocation_size);
            }
            if self.fence != vk::Fence::null() {
                self.device.destroy_fence(self.fence, None);
            }
            if self.command_buffer != vk::CommandBuffer::null()
                && self.command_pool != vk::CommandPool::null()
            {
                self.device.free_command_buffers(self.command_pool, &[self.command_buffer]);
            }
            if self.descriptor_pool != vk::DescriptorPool::null() {
                self.device.destroy_descriptor_pool(self.descriptor_pool, None);
            }
            if self.buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.buffer, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
            }
        }
    }
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
                compute_pipeline: vk::Pipeline::null(),
                pipeline_layout: vk::PipelineLayout::null(),
                descriptor_set_layout: vk::DescriptorSetLayout::null(),
                command_pool: vk::CommandPool::null(),
                next_submission_id: 1,
                submission_generation: 1,
                handle_space_exhausted: false,
                submissions: HashMap::new(),
                quarantined: HashMap::new(),
                terminal_results: HashMap::new(),
                terminal_order: VecDeque::new(),
                metrics: VulkanBackendMetrics::default(),
            }),
        }
    }

    pub fn initialize(&self) -> VulkanBackendCapabilities {
        let mut inner = self.inner.lock();
        match inner.state {
            VulkanBackendState::Ready
            | VulkanBackendState::Disabled
            | VulkanBackendState::Faulted => return inner.capabilities.clone(),
            VulkanBackendState::Uninitialized => {}
        }

        if !self.config.enable_vulkan {
            inner.state = VulkanBackendState::Disabled;
            return inner.capabilities.clone();
        }

        match self.try_init(&mut inner) {
            Ok(capabilities) => {
                inner.capabilities = capabilities;
                inner.state = VulkanBackendState::Ready;
                log::info!(
                    "vulkan-backend: initialized on device={}",
                    inner.capabilities.device_name
                );
            }
            Err(error) => {
                mark_faulted(&mut inner);
                eprintln!("vulkan-backend: initialization failed: {error:?}");
                log::error!("vulkan-backend: initialization failed: {:?}", error);
            }
        }

        inner.capabilities.clone()
    }

    pub fn state(&self) -> VulkanBackendState {
        self.inner.lock().state
    }

    pub fn metrics(&self) -> VulkanBackendMetrics {
        let mut inner = self.inner.lock();
        reap_quarantined(&mut inner);
        metrics_snapshot(&inner)
    }

    fn try_init(
        &self,
        inner: &mut VulkanBackendInner,
    ) -> Result<VulkanBackendCapabilities, Box<dyn std::error::Error>> {
        let entry = unsafe { Entry::load()? };
        let mut pending = VulkanInitGuard::new(entry);
        let app_name = CString::new("Waybroker")?;
        let engine_name = CString::new("TUFF-Xwin")?;

        let app_info = vk::ApplicationInfo::builder()
            .application_name(&app_name)
            .engine_name(&engine_name)
            .api_version(vk::API_VERSION_1_0);
        let create_info = vk::InstanceCreateInfo::builder().application_info(&app_info);

        let instance = unsafe {
            pending
                .entry
                .as_ref()
                .expect("entry retained during initialization")
                .create_instance(&create_info, None)?
        };
        pending.instance = Some(instance);
        let instance = pending.instance.as_ref().expect("instance retained during initialization");

        let physical_devices = unsafe { instance.enumerate_physical_devices()? };
        let (physical_device, queue_family_index, queue_flags) = physical_devices
            .iter()
            .find_map(|&physical_device| {
                let properties = unsafe {
                    instance.get_physical_device_queue_family_properties(physical_device)
                };
                properties.iter().enumerate().find_map(|(index, queue)| {
                    queue.queue_flags.contains(vk::QueueFlags::COMPUTE).then_some((
                        physical_device,
                        index as u32,
                        queue.queue_flags,
                    ))
                })
            })
            .ok_or("No compute-capable GPU found")?;

        let device_properties = unsafe { instance.get_physical_device_properties(physical_device) };
        let device_name = unsafe {
            CStr::from_ptr(device_properties.device_name.as_ptr()).to_string_lossy().into_owned()
        };

        let priorities = [1.0f32];
        let queue_info = [vk::DeviceQueueCreateInfo::builder()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)
            .build()];
        let device_create_info = vk::DeviceCreateInfo::builder().queue_create_infos(&queue_info);
        let device = unsafe { instance.create_device(physical_device, &device_create_info, None)? };
        pending.device = Some(device);
        let device = pending.device.as_ref().expect("device retained during initialization");
        let compute_queue = unsafe { device.get_device_queue(queue_family_index, 0) };

        let descriptor_bindings = [vk::DescriptorSetLayoutBinding::builder()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .build()];
        let descriptor_layout_info =
            vk::DescriptorSetLayoutCreateInfo::builder().bindings(&descriptor_bindings);
        pending.descriptor_set_layout =
            unsafe { device.create_descriptor_set_layout(&descriptor_layout_info, None)? };

        let layouts = [pending.descriptor_set_layout];
        let pipeline_layout_info = vk::PipelineLayoutCreateInfo::builder().set_layouts(&layouts);
        pending.pipeline_layout =
            unsafe { device.create_pipeline_layout(&pipeline_layout_info, None)? };

        let shader_code = ash::util::read_spv(&mut Cursor::new(COMPUTE_SHADER_BYTES))?;
        let shader_module_info = vk::ShaderModuleCreateInfo::builder().code(&shader_code);
        pending.shader_module = unsafe { device.create_shader_module(&shader_module_info, None)? };

        let main_cstr = CString::new("main")?;
        let shader_stage = vk::PipelineShaderStageCreateInfo::builder()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(pending.shader_module)
            .name(&main_cstr);
        let pipeline_info = vk::ComputePipelineCreateInfo::builder()
            .stage(shader_stage.build())
            .layout(pending.pipeline_layout);
        let compute_pipelines = unsafe {
            device.create_compute_pipelines(
                vk::PipelineCache::null(),
                &[pipeline_info.build()],
                None,
            )
        }
        .map_err(|(pipelines, error)| {
            for pipeline in pipelines {
                unsafe { device.destroy_pipeline(pipeline, None) };
            }
            error
        })?;
        pending.compute_pipeline =
            *compute_pipelines.first().ok_or("Vulkan returned no compute pipeline")?;

        unsafe {
            device.destroy_shader_module(pending.shader_module, None);
        }
        pending.shader_module = vk::ShaderModule::null();

        let command_pool_info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        pending.command_pool = unsafe { device.create_command_pool(&command_pool_info, None)? };

        let capabilities = VulkanBackendCapabilities {
            compute_available: true,
            transfer_available: queue_flags.contains(vk::QueueFlags::TRANSFER)
                || queue_flags.contains(vk::QueueFlags::COMPUTE),
            driver_name: "vulkan-ash-v1".to_string(),
            device_name,
        };
        pending.commit(inner, physical_device, compute_queue);
        Ok(capabilities)
    }

    pub fn submit_batch(&self, mut submission: VulkanBatchSubmission) -> VulkanBatchHandle {
        let requires_zeroize = submission.requires_zeroize;
        let host_word_bytes = submission
            .surface_words
            .as_ref()
            .and_then(|words| words.len().checked_mul(std::mem::size_of::<u32>()))
            .unwrap_or(0);

        let handle = {
            let mut inner = self.inner.lock();
            reap_quarantined(&mut inner);
            inner.metrics.total_submissions = inner.metrics.total_submissions.saturating_add(1);

            if let Some(reason) = self.admission_fallback_reason(&inner, &submission) {
                inner.metrics.cpu_fallback_submissions =
                    inner.metrics.cpu_fallback_submissions.saturating_add(1);
                allocate_handle(
                    &mut inner,
                    submission.workload,
                    VulkanExecutionPath::CpuFallback,
                    Some(reason),
                )
            } else {
                let mut handle = allocate_handle(
                    &mut inner,
                    submission.workload,
                    VulkanExecutionPath::Vulkan,
                    None,
                );
                if handle.id == 0 {
                    handle.path = VulkanExecutionPath::CpuFallback;
                    handle.fallback_reason = Some(VulkanFallbackReason::SubmissionRejected);
                    inner.metrics.cpu_fallback_submissions =
                        inner.metrics.cpu_fallback_submissions.saturating_add(1);
                    handle
                } else {
                    let timeout = effective_timeout(submission.timeout, self.config.submit_timeout);
                    match self.try_submit(&inner, &submission, timeout) {
                        Ok(stored) => {
                            let allocation_size = stored.allocation_size;
                            let key = handle_key(handle);
                            inner.submissions.insert(key, stored);
                            inner.metrics.vulkan_submissions =
                                inner.metrics.vulkan_submissions.saturating_add(1);
                            inner.metrics.current_gpu_allocation_bytes = inner
                                .metrics
                                .current_gpu_allocation_bytes
                                .saturating_add(allocation_size);
                            inner.metrics.peak_gpu_allocation_bytes = inner
                                .metrics
                                .peak_gpu_allocation_bytes
                                .max(inner.metrics.current_gpu_allocation_bytes);
                            handle
                        }
                        Err(error) => {
                            if error.is_device_lost() {
                                mark_faulted(&mut inner);
                            }
                            log::error!("vulkan-backend: submission failed: {}", error);
                            handle.path = VulkanExecutionPath::CpuFallback;
                            handle.fallback_reason = Some(if error.is_device_lost() {
                                VulkanFallbackReason::DriverUnavailable
                            } else {
                                VulkanFallbackReason::SubmissionRejected
                            });
                            inner.metrics.cpu_fallback_submissions =
                                inner.metrics.cpu_fallback_submissions.saturating_add(1);
                            handle
                        }
                    }
                }
            }
        };

        if requires_zeroize {
            zeroize_surface_words(&mut submission.surface_words);
            if host_word_bytes != 0 {
                let mut inner = self.inner.lock();
                inner.metrics.zeroized_host_bytes =
                    inner.metrics.zeroized_host_bytes.saturating_add(host_word_bytes as u64);
            }
        }

        handle
    }

    fn admission_fallback_reason(
        &self,
        inner: &VulkanBackendInner,
        submission: &VulkanBatchSubmission,
    ) -> Option<VulkanFallbackReason> {
        if submission.payload_len == 0 || submission.payload_len > MAX_GPU_PAYLOAD_BYTES {
            return Some(VulkanFallbackReason::SubmissionRejected);
        }

        if let Some(words) = submission.surface_words.as_ref() {
            let Some(word_bytes) = words.len().checked_mul(std::mem::size_of::<u32>()) else {
                return Some(VulkanFallbackReason::SubmissionRejected);
            };
            if words.is_empty()
                || words.len() > MAX_SURFACE_WORDS
                || word_bytes != submission.payload_len
            {
                return Some(VulkanFallbackReason::SubmissionRejected);
            }
        }

        if submission.workload == VulkanWorkloadClass::PacketPreclassification
            && submission.payload_len < self.config.packet_preclassification_min_batch_bytes
        {
            return Some(VulkanFallbackReason::BelowBatchThreshold);
        }
        if !submission.allows_gpu {
            return Some(VulkanFallbackReason::DisabledByPolicy);
        }
        match inner.state {
            VulkanBackendState::Uninitialized => {
                return Some(VulkanFallbackReason::NotInitialized);
            }
            VulkanBackendState::Disabled => {
                return Some(VulkanFallbackReason::DisabledByPolicy);
            }
            VulkanBackendState::Faulted => {
                return Some(VulkanFallbackReason::DriverUnavailable);
            }
            VulkanBackendState::Ready => {}
        }
        if active_gpu_resource_count(inner) >= MAX_PENDING_GPU_SUBMISSIONS {
            return Some(VulkanFallbackReason::SubmissionRejected);
        }
        if !inner.capabilities.compute_available || inner.device.is_none() {
            return Some(VulkanFallbackReason::CapabilityUnavailable);
        }

        // The current shader consumes only explicit u32 metadata. Callers that do not
        // provide such metadata already execute their authoritative CPU/SIMD path, so
        // they remain on that fallback instead of dispatching uninitialized GPU memory.
        if submission.surface_words.is_none() {
            return Some(VulkanFallbackReason::CapabilityUnavailable);
        }

        None
    }

    fn try_submit(
        &self,
        inner: &VulkanBackendInner,
        submission: &VulkanBatchSubmission,
        timeout: Duration,
    ) -> Result<VulkanStoredSubmission, SubmitError> {
        let device = inner.device.as_ref().ok_or(SubmitError::Contract("device missing"))?;
        let words = submission
            .surface_words
            .as_ref()
            .ok_or(SubmitError::Contract("metadata words missing"))?;
        let payload_size = vk::DeviceSize::try_from(submission.payload_len)
            .map_err(|_| SubmitError::Contract("payload length does not fit DeviceSize"))?;
        let mut pending =
            PendingGpuResources::new(device, inner.command_pool, submission.requires_zeroize);

        let buffer_info = vk::BufferCreateInfo::builder()
            .size(payload_size)
            .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        pending.buffer = unsafe { device.create_buffer(&buffer_info, None)? };
        let memory_requirements = unsafe { device.get_buffer_memory_requirements(pending.buffer) };
        if inner
            .metrics
            .current_gpu_allocation_bytes
            .checked_add(memory_requirements.size)
            .is_none_or(|total| total > MAX_TOTAL_GPU_ALLOCATION_BYTES)
        {
            return Err(SubmitError::Contract("aggregate GPU allocation budget exceeded"));
        }

        let instance = inner.instance.as_ref().ok_or(SubmitError::Contract("instance missing"))?;
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(inner.physical_device) };
        let memory_type_index = (0..memory_properties.memory_type_count)
            .find(|&index| {
                (memory_requirements.memory_type_bits & (1 << index)) != 0
                    && memory_properties.memory_types[index as usize].property_flags.contains(
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )
            })
            .ok_or(SubmitError::Contract("no host-visible coherent memory type"))?;
        let allocation_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(memory_requirements.size)
            .memory_type_index(memory_type_index);
        pending.memory = unsafe { device.allocate_memory(&allocation_info, None)? };
        pending.allocation_size = memory_requirements.size;
        unsafe { device.bind_buffer_memory(pending.buffer, pending.memory, 0)? };

        let mapped = unsafe {
            device.map_memory(pending.memory, 0, payload_size, vk::MemoryMapFlags::empty())?
        };
        let bytes = words
            .len()
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or(SubmitError::Contract("metadata byte length overflow"))?;
        unsafe {
            std::ptr::copy_nonoverlapping(words.as_ptr().cast::<u8>(), mapped.cast::<u8>(), bytes);
            device.unmap_memory(pending.memory);
        }

        let command_buffer_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(inner.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffers = unsafe { device.allocate_command_buffers(&command_buffer_info)? };
        pending.command_buffer = *command_buffers
            .first()
            .ok_or(SubmitError::Contract("Vulkan returned no command buffer"))?;

        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            device.begin_command_buffer(pending.command_buffer, &begin_info)?;
            device.cmd_bind_pipeline(
                pending.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                inner.compute_pipeline,
            );
        }

        let descriptor_pool_sizes = [vk::DescriptorPoolSize::builder()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .build()];
        let descriptor_pool_info =
            vk::DescriptorPoolCreateInfo::builder().max_sets(1).pool_sizes(&descriptor_pool_sizes);
        pending.descriptor_pool =
            unsafe { device.create_descriptor_pool(&descriptor_pool_info, None)? };
        let layouts = [inner.descriptor_set_layout];
        let descriptor_set_info = vk::DescriptorSetAllocateInfo::builder()
            .descriptor_pool(pending.descriptor_pool)
            .set_layouts(&layouts);
        let descriptor_sets = unsafe { device.allocate_descriptor_sets(&descriptor_set_info)? };
        let descriptor_set = *descriptor_sets
            .first()
            .ok_or(SubmitError::Contract("Vulkan returned no descriptor set"))?;
        let descriptor_buffer_info = [vk::DescriptorBufferInfo::builder()
            .buffer(pending.buffer)
            .offset(0)
            .range(payload_size)
            .build()];
        let descriptor_writes = [vk::WriteDescriptorSet::builder()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&descriptor_buffer_info)
            .build()];
        unsafe {
            device.update_descriptor_sets(&descriptor_writes, &[]);
            device.cmd_bind_descriptor_sets(
                pending.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                inner.pipeline_layout,
                0,
                &[descriptor_set],
                &[],
            );
        }

        let element_count = submission.payload_len / std::mem::size_of::<u32>();
        let group_count = element_count
            .checked_add(63)
            .ok_or(SubmitError::Contract("dispatch group count overflow"))?
            / 64;
        let group_count = u32::try_from(group_count)
            .map_err(|_| SubmitError::Contract("dispatch group count exceeds u32"))?;
        unsafe {
            device.cmd_dispatch(pending.command_buffer, group_count, 1, 1);
            device.end_command_buffer(pending.command_buffer)?;
        }

        let fence_info = vk::FenceCreateInfo::builder();
        pending.fence = unsafe { device.create_fence(&fence_info, None)? };
        let command_buffers = [pending.command_buffer];
        let submit_info = [vk::SubmitInfo::builder().command_buffers(&command_buffers).build()];
        unsafe {
            device.queue_submit(inner.compute_queue, &submit_info, pending.fence)?;
        }

        let now = Instant::now();
        let deadline = now.checked_add(timeout).unwrap_or(now);
        Ok(pending.into_stored(submission.workload, deadline))
    }

    pub fn poll_completion(&self, handle: VulkanBatchHandle) -> VulkanPollStatus {
        if handle.path == VulkanExecutionPath::CpuFallback {
            return VulkanPollStatus::Completed;
        }
        let key = handle_key(handle);
        let mut inner = self.inner.lock();
        reap_quarantined(&mut inner);
        if let Some(record) = inner.terminal_results.get(&key) {
            return record.status;
        }

        let Some(submission) = inner.submissions.get(&key) else {
            return VulkanPollStatus::Missing;
        };
        if submission.completed_at.is_some() {
            return VulkanPollStatus::Completed;
        }
        let fence = submission.fence;
        let deadline = submission.deadline;
        let Some(device) = inner.device.clone() else {
            terminalize_submission(
                &mut inner,
                key,
                VulkanPollStatus::Missing,
                VulkanFallbackReason::DriverUnavailable,
                Instant::now(),
            );
            mark_faulted(&mut inner);
            return VulkanPollStatus::Missing;
        };

        let now = Instant::now();
        match unsafe { device.get_fence_status(fence) } {
            Ok(true) => {
                if let Some(submission) = inner.submissions.get_mut(&key) {
                    submission.completed_at = Some(now);
                }
                VulkanPollStatus::Completed
            }
            Ok(false) if now >= deadline => {
                terminalize_submission(
                    &mut inner,
                    key,
                    VulkanPollStatus::TimedOut,
                    VulkanFallbackReason::Timeout,
                    now,
                );
                inner.metrics.timed_out_submissions =
                    inner.metrics.timed_out_submissions.saturating_add(1);
                mark_faulted(&mut inner);
                VulkanPollStatus::TimedOut
            }
            Ok(false) => VulkanPollStatus::Pending,
            Err(error) => {
                log::error!(
                    "vulkan-backend: fence status failed for id={} generation={}: {:?}",
                    handle.id,
                    handle.generation,
                    error
                );
                terminalize_submission(
                    &mut inner,
                    key,
                    VulkanPollStatus::Missing,
                    VulkanFallbackReason::DriverUnavailable,
                    now,
                );
                mark_faulted(&mut inner);
                VulkanPollStatus::Missing
            }
        }
    }

    pub async fn wait_for_completion(&self, handle: VulkanBatchHandle) -> VulkanBatchResult {
        if handle.path == VulkanExecutionPath::CpuFallback {
            return fallback_result(handle, handle.fallback_reason, Instant::now());
        }

        loop {
            match self.poll_completion(handle) {
                VulkanPollStatus::Completed => {
                    let mut inner = self.inner.lock();
                    if let Some(record) = take_terminal_result(&mut inner, handle_key(handle)) {
                        return fallback_result(
                            handle,
                            Some(record.fallback_reason),
                            record.completed_at,
                        );
                    }
                    let key = handle_key(handle);
                    let Some(submission) = inner.submissions.remove(&key) else {
                        return fallback_result(
                            handle,
                            Some(VulkanFallbackReason::SubmissionRejected),
                            Instant::now(),
                        );
                    };
                    let workload = submission.workload;
                    let completed_at = submission.completed_at.unwrap_or_else(Instant::now);
                    match release_gpu_submission(&mut inner, submission) {
                        Ok(()) => {}
                        Err(submission) => {
                            inner.quarantined.insert(key, submission);
                            mark_faulted(&mut inner);
                        }
                    }
                    inner.metrics.completed_vulkan_submissions =
                        inner.metrics.completed_vulkan_submissions.saturating_add(1);
                    return VulkanBatchResult {
                        handle,
                        path: VulkanExecutionPath::Vulkan,
                        workload,
                        fallback_reason: None,
                        completed_at,
                    };
                }
                VulkanPollStatus::TimedOut | VulkanPollStatus::Missing => {
                    let mut inner = self.inner.lock();
                    if let Some(record) = take_terminal_result(&mut inner, handle_key(handle)) {
                        return fallback_result(
                            handle,
                            Some(record.fallback_reason),
                            record.completed_at,
                        );
                    }
                    return fallback_result(
                        handle,
                        Some(VulkanFallbackReason::SubmissionRejected),
                        Instant::now(),
                    );
                }
                VulkanPollStatus::Pending => {
                    sleep(Duration::from_millis(MIN_PENDING_POLL_MS)).await;
                }
            }
        }
    }

    pub fn retire_submission(&self, handle: VulkanBatchHandle) {
        if handle.path == VulkanExecutionPath::CpuFallback {
            return;
        }
        let key = handle_key(handle);
        let mut inner = self.inner.lock();
        reap_quarantined(&mut inner);
        if let Some(submission) = inner.submissions.remove(&key) {
            let signaled = match inner.device.clone() {
                Some(device) => match unsafe { device.get_fence_status(submission.fence) } {
                    Ok(signaled) => signaled,
                    Err(_) => {
                        mark_faulted(&mut inner);
                        false
                    }
                },
                None => {
                    mark_faulted(&mut inner);
                    false
                }
            };
            if signaled {
                if let Err(submission) = release_gpu_submission(&mut inner, submission) {
                    inner.quarantined.insert(key, submission);
                    mark_faulted(&mut inner);
                }
            } else {
                inner.quarantined.insert(key, submission);
            }
        }
        let _ = take_terminal_result(&mut inner, key);
    }

    /// Refines screenshot pixels using the process-wide acceleration policy.
    /// Swaps R and B channels (BGRA <-> RGBA) with exact scalar semantics.
    pub fn refine_screenshot_pixels(&self, pixels: &mut [u32]) {
        #[cfg(target_arch = "x86_64")]
        {
            if global_accel_policy().avx2_available {
                unsafe {
                    self.refine_pixels_avx2(pixels);
                }
                return;
            }
        }
        refine_pixels_portable(pixels);
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn refine_pixels_avx2(&self, pixels: &mut [u32]) {
        let mask = _mm256_setr_epi8(
            2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15, 2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8,
            11, 14, 13, 12, 15,
        );
        for chunk in pixels.chunks_mut(8) {
            if chunk.len() < 8 {
                refine_pixels_portable(chunk);
                break;
            }
            let ptr = chunk.as_mut_ptr().cast::<__m256i>();
            let data = _mm256_loadu_si256(ptr);
            let shuffled = _mm256_shuffle_epi8(data, mask);
            _mm256_storeu_si256(ptr, shuffled);
        }
    }
}

fn zeroize_surface_words(words: &mut Option<Vec<u32>>) {
    if let Some(words) = words.as_mut() {
        words.zeroize();
    }
}

fn refine_pixels_portable(pixels: &mut [u32]) {
    for pixel in pixels {
        let high = (*pixel >> 16) & 0xFF;
        let low = *pixel & 0xFF;
        *pixel = (*pixel & 0xFF00_FF00) | (low << 16) | high;
    }
}

fn effective_timeout(requested: Duration, configured: Duration) -> Duration {
    let hard_max = Duration::from_millis(MAX_TIMEOUT_MS);
    let configured = if configured.is_zero() {
        Duration::from_millis(DEFAULT_TIMEOUT_MS)
    } else {
        configured.min(hard_max)
    };
    if requested.is_zero() {
        configured
    } else {
        requested.min(configured).min(hard_max)
    }
}

fn handle_key(handle: VulkanBatchHandle) -> SubmissionKey {
    SubmissionKey { generation: handle.generation, id: handle.id }
}

fn allocate_handle(
    inner: &mut VulkanBackendInner,
    workload: VulkanWorkloadClass,
    path: VulkanExecutionPath,
    fallback_reason: Option<VulkanFallbackReason>,
) -> VulkanBatchHandle {
    if inner.handle_space_exhausted {
        return VulkanBatchHandle { id: 0, generation: 0, workload, path, fallback_reason };
    }
    let id = inner.next_submission_id;
    let generation = inner.submission_generation;
    if inner.next_submission_id == u64::MAX {
        if inner.submission_generation == u64::MAX {
            inner.handle_space_exhausted = true;
        } else {
            inner.next_submission_id = 1;
            inner.submission_generation += 1;
        }
    } else {
        inner.next_submission_id += 1;
    }
    VulkanBatchHandle { id, generation, workload, path, fallback_reason }
}

fn active_gpu_resource_count(inner: &VulkanBackendInner) -> usize {
    inner.submissions.len().saturating_add(inner.quarantined.len())
}

fn metrics_snapshot(inner: &VulkanBackendInner) -> VulkanBackendMetrics {
    let mut metrics = inner.metrics;
    metrics.pending_gpu_submissions = inner.submissions.len();
    metrics.quarantined_gpu_submissions = inner.quarantined.len();
    metrics
}

fn mark_faulted(inner: &mut VulkanBackendInner) {
    if inner.state != VulkanBackendState::Faulted {
        inner.metrics.driver_faults = inner.metrics.driver_faults.saturating_add(1);
    }
    inner.state = VulkanBackendState::Faulted;
    inner.capabilities.compute_available = false;
    inner.capabilities.transfer_available = false;
}

fn terminalize_submission(
    inner: &mut VulkanBackendInner,
    key: SubmissionKey,
    status: VulkanPollStatus,
    fallback_reason: VulkanFallbackReason,
    completed_at: Instant,
) {
    if let Some(submission) = inner.submissions.remove(&key) {
        inner.quarantined.insert(key, submission);
        inner.metrics.cpu_fallback_submissions =
            inner.metrics.cpu_fallback_submissions.saturating_add(1);
    }
    insert_terminal_result(
        inner,
        key,
        VulkanTerminalRecord { status, fallback_reason, completed_at },
    );
}

fn insert_terminal_result(
    inner: &mut VulkanBackendInner,
    key: SubmissionKey,
    record: VulkanTerminalRecord,
) {
    if !inner.terminal_results.contains_key(&key) {
        while inner.terminal_results.len() >= MAX_TERMINAL_RESULTS {
            let Some(oldest) = inner.terminal_order.pop_front() else {
                break;
            };
            inner.terminal_results.remove(&oldest);
        }
        inner.terminal_order.push_back(key);
    }
    inner.terminal_results.insert(key, record);
}

fn take_terminal_result(
    inner: &mut VulkanBackendInner,
    key: SubmissionKey,
) -> Option<VulkanTerminalRecord> {
    let record = inner.terminal_results.remove(&key)?;
    inner.terminal_order.retain(|candidate| *candidate != key);
    Some(record)
}

fn fallback_result(
    handle: VulkanBatchHandle,
    reason: Option<VulkanFallbackReason>,
    completed_at: Instant,
) -> VulkanBatchResult {
    VulkanBatchResult {
        handle,
        path: VulkanExecutionPath::CpuFallback,
        workload: handle.workload,
        fallback_reason: reason.or(handle.fallback_reason),
        completed_at,
    }
}

fn reap_quarantined(inner: &mut VulkanBackendInner) {
    let Some(device) = inner.device.clone() else {
        return;
    };
    let keys: Vec<SubmissionKey> = inner.quarantined.keys().copied().collect();
    for key in keys {
        let Some(fence) = inner.quarantined.get(&key).map(|submission| submission.fence) else {
            continue;
        };
        match unsafe { device.get_fence_status(fence) } {
            Ok(true) => {
                let Some(submission) = inner.quarantined.remove(&key) else {
                    continue;
                };
                if let Err(submission) = release_gpu_submission(inner, submission) {
                    inner.quarantined.insert(key, submission);
                    mark_faulted(inner);
                }
            }
            Ok(false) => {}
            Err(_) => mark_faulted(inner),
        }
    }
}

fn release_gpu_submission(
    inner: &mut VulkanBackendInner,
    submission: VulkanStoredSubmission,
) -> Result<(), VulkanStoredSubmission> {
    let Some(device) = inner.device.clone() else {
        return Err(submission);
    };
    if submission.requires_zeroize
        && zeroize_device_memory(&device, submission.memory, submission.allocation_size).is_err()
    {
        return Err(submission);
    }

    unsafe {
        if submission.fence != vk::Fence::null() {
            device.destroy_fence(submission.fence, None);
        }
        if submission.command_buffer != vk::CommandBuffer::null()
            && inner.command_pool != vk::CommandPool::null()
        {
            device.free_command_buffers(inner.command_pool, &[submission.command_buffer]);
        }
        if submission.descriptor_pool != vk::DescriptorPool::null() {
            device.destroy_descriptor_pool(submission.descriptor_pool, None);
        }
        if submission.buffer != vk::Buffer::null() {
            device.destroy_buffer(submission.buffer, None);
        }
        if submission.memory != vk::DeviceMemory::null() {
            device.free_memory(submission.memory, None);
        }
    }

    inner.metrics.current_gpu_allocation_bytes =
        inner.metrics.current_gpu_allocation_bytes.saturating_sub(submission.allocation_size);
    if submission.requires_zeroize {
        inner.metrics.zeroized_device_bytes =
            inner.metrics.zeroized_device_bytes.saturating_add(submission.allocation_size);
    }
    Ok(())
}

fn zeroize_device_memory(
    device: &Device,
    memory: vk::DeviceMemory,
    allocation_size: vk::DeviceSize,
) -> Result<(), vk::Result> {
    if memory == vk::DeviceMemory::null() || allocation_size == 0 {
        return Ok(());
    }
    let length =
        usize::try_from(allocation_size).map_err(|_| vk::Result::ERROR_OUT_OF_HOST_MEMORY)?;
    let pointer =
        unsafe { device.map_memory(memory, 0, allocation_size, vk::MemoryMapFlags::empty())? };
    unsafe {
        std::ptr::write_bytes(pointer.cast::<u8>(), 0, length);
        device.unmap_memory(memory);
    }
    Ok(())
}

impl Drop for VulkanBackendInner {
    fn drop(&mut self) {
        if let Some(device) = self.device.clone() {
            unsafe {
                let _ = device.device_wait_idle();
            }

            let submissions = std::mem::take(&mut self.submissions);
            let quarantined = std::mem::take(&mut self.quarantined);
            for submission in submissions.into_values().chain(quarantined.into_values()) {
                if submission.requires_zeroize {
                    let _ = zeroize_device_memory(
                        &device,
                        submission.memory,
                        submission.allocation_size,
                    );
                }
                unsafe {
                    if submission.fence != vk::Fence::null() {
                        device.destroy_fence(submission.fence, None);
                    }
                    if submission.command_buffer != vk::CommandBuffer::null()
                        && self.command_pool != vk::CommandPool::null()
                    {
                        device
                            .free_command_buffers(self.command_pool, &[submission.command_buffer]);
                    }
                    if submission.descriptor_pool != vk::DescriptorPool::null() {
                        device.destroy_descriptor_pool(submission.descriptor_pool, None);
                    }
                    if submission.buffer != vk::Buffer::null() {
                        device.destroy_buffer(submission.buffer, None);
                    }
                    if submission.memory != vk::DeviceMemory::null() {
                        device.free_memory(submission.memory, None);
                    }
                }
            }

            unsafe {
                if self.command_pool != vk::CommandPool::null() {
                    device.destroy_command_pool(self.command_pool, None);
                }
                if self.compute_pipeline != vk::Pipeline::null() {
                    device.destroy_pipeline(self.compute_pipeline, None);
                }
                if self.pipeline_layout != vk::PipelineLayout::null() {
                    device.destroy_pipeline_layout(self.pipeline_layout, None);
                }
                if self.descriptor_set_layout != vk::DescriptorSetLayout::null() {
                    device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> VulkanBackendConfig {
        VulkanBackendConfig {
            enable_vulkan: false,
            packet_preclassification_min_batch_bytes: DEFAULT_PACKET_MIN_BATCH_BYTES,
            submit_timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }

    fn submission(
        workload: VulkanWorkloadClass,
        payload_len: usize,
        surface_words: Option<Vec<u32>>,
    ) -> VulkanBatchSubmission {
        VulkanBatchSubmission {
            workload,
            payload_len,
            surface_words,
            timeout: Duration::from_millis(50),
            requires_zeroize: false,
            allows_gpu: true,
        }
    }

    fn fake_stored_submission(workload: VulkanWorkloadClass) -> VulkanStoredSubmission {
        VulkanStoredSubmission {
            workload,
            fence: vk::Fence::null(),
            buffer: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            command_buffer: vk::CommandBuffer::null(),
            allocation_size: 0,
            deadline: Instant::now(),
            completed_at: None,
            requires_zeroize: false,
        }
    }

    #[test]
    fn completion_refine_screenshot_pixels_matches_portable_semantics() {
        let backend = VulkanBackend::new(test_config());
        let mut expected = vec![0xFF33_2211u32; 101];
        let mut actual = expected.clone();
        refine_pixels_portable(&mut expected);
        backend.refine_screenshot_pixels(&mut actual);
        assert_eq!(actual, expected);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn completion_avx2_refine_matches_portable_when_available() {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return;
        }
        let backend = VulkanBackend::new(test_config());
        let mut expected: Vec<u32> =
            (0..257).map(|value| 0xFF00_0000 | ((value * 17) & 0x00FF_FFFF)).collect();
        let mut actual = expected.clone();
        refine_pixels_portable(&mut expected);
        unsafe { backend.refine_pixels_avx2(&mut actual) };
        assert_eq!(actual, expected);
    }

    #[test]
    fn completion_initialize_is_idempotent_when_disabled() {
        let backend = VulkanBackend::new(test_config());
        assert!(!backend.initialize().compute_available);
        assert_eq!(backend.state(), VulkanBackendState::Disabled);
        assert!(!backend.initialize().compute_available);
        assert_eq!(backend.state(), VulkanBackendState::Disabled);
    }

    #[tokio::test]
    async fn completion_disabled_backend_preserves_workload_and_reason() {
        let backend = VulkanBackend::new(test_config());
        backend.initialize();
        let handle = backend.submit_batch(submission(
            VulkanWorkloadClass::SceneComposition,
            4,
            Some(vec![1]),
        ));
        let result = backend.wait_for_completion(handle).await;
        assert_eq!(result.path, VulkanExecutionPath::CpuFallback);
        assert_eq!(result.workload, VulkanWorkloadClass::SceneComposition);
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::DisabledByPolicy));
        assert_eq!(backend.metrics().pending_gpu_submissions, 0);
    }

    #[tokio::test]
    async fn completion_submit_before_initialize_reports_not_initialized() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        let handle =
            backend.submit_batch(submission(VulkanWorkloadClass::BulkPrefilter, 4, Some(vec![1])));
        let result = backend.wait_for_completion(handle).await;
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::NotInitialized));
    }

    #[tokio::test]
    async fn completion_allows_gpu_false_never_enters_gpu_state() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        let mut request = submission(VulkanWorkloadClass::BulkPrefilter, 4, Some(vec![1]));
        request.allows_gpu = false;
        let result = backend.wait_for_completion(backend.submit_batch(request)).await;
        assert_eq!(result.path, VulkanExecutionPath::CpuFallback);
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::DisabledByPolicy));
        assert_eq!(backend.metrics().pending_gpu_submissions, 0);
    }

    #[tokio::test]
    async fn completion_oversized_payload_is_rejected_before_gpu_allocation() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        let request =
            submission(VulkanWorkloadClass::BulkPrefilter, MAX_GPU_PAYLOAD_BYTES + 1, None);
        let result = backend.wait_for_completion(backend.submit_batch(request)).await;
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::SubmissionRejected));
        assert_eq!(backend.metrics().current_gpu_allocation_bytes, 0);
    }

    #[tokio::test]
    async fn completion_surface_word_payload_mismatch_is_rejected() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        let result = backend
            .wait_for_completion(backend.submit_batch(submission(
                VulkanWorkloadClass::SceneComposition,
                8,
                Some(vec![1]),
            )))
            .await;
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::SubmissionRejected));
    }

    #[tokio::test]
    async fn completion_surface_word_count_is_bounded() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        let words = vec![0u32; MAX_SURFACE_WORDS + 1];
        let bytes = words.len() * std::mem::size_of::<u32>();
        let result = backend
            .wait_for_completion(backend.submit_batch(submission(
                VulkanWorkloadClass::SceneComposition,
                bytes,
                Some(words),
            )))
            .await;
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::SubmissionRejected));
    }

    #[tokio::test]
    async fn completion_packet_preclassification_honors_batch_threshold() {
        let backend = VulkanBackend::new(VulkanBackendConfig {
            enable_vulkan: true,
            packet_preclassification_min_batch_bytes: 1024,
            submit_timeout: Duration::from_millis(250),
        });
        let result = backend
            .wait_for_completion(backend.submit_batch(submission(
                VulkanWorkloadClass::PacketPreclassification,
                4,
                Some(vec![1]),
            )))
            .await;
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::BelowBatchThreshold));
    }

    #[tokio::test]
    async fn completion_ready_backend_without_metadata_uses_cpu_fallback() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        {
            let mut inner = backend.inner.lock();
            inner.state = VulkanBackendState::Ready;
            inner.capabilities.compute_available = true;
        }
        let result = backend
            .wait_for_completion(backend.submit_batch(submission(
                VulkanWorkloadClass::ScreenshotRefine,
                4096,
                None,
            )))
            .await;
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::CapabilityUnavailable));
        assert_eq!(backend.metrics().pending_gpu_submissions, 0);
    }

    #[tokio::test]
    async fn completion_faulted_backend_fails_soft_without_gpu_state() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        {
            let mut inner = backend.inner.lock();
            inner.state = VulkanBackendState::Faulted;
        }
        let result = backend
            .wait_for_completion(backend.submit_batch(submission(
                VulkanWorkloadClass::BulkPrefilter,
                4,
                Some(vec![1]),
            )))
            .await;
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::DriverUnavailable));
        assert_eq!(backend.metrics().pending_gpu_submissions, 0);
    }

    #[test]
    fn completion_pending_gpu_resource_budget_backpressures() {
        let backend =
            VulkanBackend::new(VulkanBackendConfig { enable_vulkan: true, ..test_config() });
        {
            let mut inner = backend.inner.lock();
            inner.state = VulkanBackendState::Ready;
            inner.capabilities.compute_available = true;
            for id in 1..=MAX_PENDING_GPU_SUBMISSIONS as u64 {
                inner.submissions.insert(
                    SubmissionKey { generation: 99, id },
                    fake_stored_submission(VulkanWorkloadClass::SceneComposition),
                );
            }
        }
        let handle = backend.submit_batch(submission(
            VulkanWorkloadClass::SceneComposition,
            4,
            Some(vec![1]),
        ));
        assert_eq!(handle.path, VulkanExecutionPath::CpuFallback);
        assert_eq!(handle.fallback_reason, Some(VulkanFallbackReason::SubmissionRejected));
        assert_eq!(backend.inner.lock().submissions.len(), MAX_PENDING_GPU_SUBMISSIONS);
    }

    #[test]
    fn completion_handle_wrap_changes_generation_and_never_aliases() {
        let backend = VulkanBackend::new(test_config());
        let mut inner = backend.inner.lock();
        inner.next_submission_id = u64::MAX;
        inner.submission_generation = 7;
        let first = allocate_handle(
            &mut inner,
            VulkanWorkloadClass::AuditScan,
            VulkanExecutionPath::CpuFallback,
            None,
        );
        let second = allocate_handle(
            &mut inner,
            VulkanWorkloadClass::AuditScan,
            VulkanExecutionPath::CpuFallback,
            None,
        );
        assert_eq!(first.id, u64::MAX);
        assert_eq!(first.generation, 7);
        assert_eq!(second.id, 1);
        assert_eq!(second.generation, 8);
        assert_ne!(handle_key(first), handle_key(second));
    }

    #[test]
    fn completion_handle_space_exhaustion_never_reuses_last_identity() {
        let backend = VulkanBackend::new(test_config());
        let mut inner = backend.inner.lock();
        inner.next_submission_id = u64::MAX;
        inner.submission_generation = u64::MAX;
        let last = allocate_handle(
            &mut inner,
            VulkanWorkloadClass::AuditScan,
            VulkanExecutionPath::CpuFallback,
            None,
        );
        let exhausted = allocate_handle(
            &mut inner,
            VulkanWorkloadClass::AuditScan,
            VulkanExecutionPath::CpuFallback,
            Some(VulkanFallbackReason::SubmissionRejected),
        );
        assert_eq!(last.id, u64::MAX);
        assert_eq!(last.generation, u64::MAX);
        assert_eq!(exhausted.id, 0);
        assert_eq!(exhausted.generation, 0);
    }

    #[test]
    fn completion_terminal_result_history_is_bounded() {
        let backend = VulkanBackend::new(test_config());
        let mut inner = backend.inner.lock();
        for id in 1..=(MAX_TERMINAL_RESULTS as u64 + 17) {
            insert_terminal_result(
                &mut inner,
                SubmissionKey { generation: 1, id },
                VulkanTerminalRecord {
                    status: VulkanPollStatus::TimedOut,
                    fallback_reason: VulkanFallbackReason::Timeout,
                    completed_at: Instant::now(),
                },
            );
        }
        assert_eq!(inner.terminal_results.len(), MAX_TERMINAL_RESULTS);
        assert_eq!(inner.terminal_order.len(), MAX_TERMINAL_RESULTS);
    }

    #[test]
    fn completion_effective_timeout_is_bounded() {
        assert_eq!(
            effective_timeout(Duration::ZERO, Duration::from_millis(250)),
            Duration::from_millis(250)
        );
        assert_eq!(
            effective_timeout(Duration::from_secs(60), Duration::from_secs(60)),
            Duration::from_millis(MAX_TIMEOUT_MS)
        );
        assert_eq!(
            effective_timeout(Duration::from_millis(50), Duration::from_millis(250)),
            Duration::from_millis(50)
        );
    }

    #[test]
    fn completion_requires_zeroize_clears_host_surface_words() {
        let mut words = Some(vec![0xDEAD_BEEFu32, 0xA5A5_5A5A]);
        let byte_len = words.as_ref().unwrap().len() * std::mem::size_of::<u32>();
        zeroize_surface_words(&mut words);
        assert_eq!(byte_len, 8);
        assert!(words.unwrap().iter().all(|word| *word == 0));
    }

    #[test]
    fn completion_timeout_terminalization_quarantines_resources_without_destroying_them() {
        let backend = VulkanBackend::new(test_config());
        let mut inner = backend.inner.lock();
        let handle = allocate_handle(
            &mut inner,
            VulkanWorkloadClass::SceneComposition,
            VulkanExecutionPath::Vulkan,
            None,
        );
        let key = handle_key(handle);
        inner
            .submissions
            .insert(key, fake_stored_submission(VulkanWorkloadClass::SceneComposition));
        terminalize_submission(
            &mut inner,
            key,
            VulkanPollStatus::TimedOut,
            VulkanFallbackReason::Timeout,
            Instant::now(),
        );
        assert!(!inner.submissions.contains_key(&key));
        assert!(inner.quarantined.contains_key(&key));
        assert_eq!(
            inner.terminal_results.get(&key).map(|record| record.status),
            Some(VulkanPollStatus::TimedOut)
        );
    }

    #[test]
    fn completion_retire_of_unverifiable_gpu_submission_quarantines_it() {
        let backend = VulkanBackend::new(test_config());
        let handle = {
            let mut inner = backend.inner.lock();
            let handle = allocate_handle(
                &mut inner,
                VulkanWorkloadClass::BulkPrefilter,
                VulkanExecutionPath::Vulkan,
                None,
            );
            inner.submissions.insert(handle_key(handle), fake_stored_submission(handle.workload));
            handle
        };
        backend.retire_submission(handle);
        let inner = backend.inner.lock();
        assert!(!inner.submissions.contains_key(&handle_key(handle)));
        assert!(inner.quarantined.contains_key(&handle_key(handle)));
        assert_eq!(inner.state, VulkanBackendState::Faulted);
    }

    #[tokio::test]
    async fn completion_missing_gpu_handle_preserves_original_workload() {
        let backend = VulkanBackend::new(test_config());
        let handle = {
            let mut inner = backend.inner.lock();
            allocate_handle(
                &mut inner,
                VulkanWorkloadClass::MaintenanceHashing,
                VulkanExecutionPath::Vulkan,
                None,
            )
        };
        let result = backend.wait_for_completion(handle).await;
        assert_eq!(result.path, VulkanExecutionPath::CpuFallback);
        assert_eq!(result.workload, VulkanWorkloadClass::MaintenanceHashing);
        assert_eq!(result.fallback_reason, Some(VulkanFallbackReason::SubmissionRejected));
    }

    #[test]
    fn completion_metrics_report_active_and_quarantined_resources_separately() {
        let backend = VulkanBackend::new(test_config());
        {
            let mut inner = backend.inner.lock();
            inner.submissions.insert(
                SubmissionKey { generation: 1, id: 1 },
                fake_stored_submission(VulkanWorkloadClass::SceneComposition),
            );
            inner.quarantined.insert(
                SubmissionKey { generation: 1, id: 2 },
                fake_stored_submission(VulkanWorkloadClass::BulkPrefilter),
            );
        }
        let metrics = backend.metrics();
        assert_eq!(metrics.pending_gpu_submissions, 1);
        assert_eq!(metrics.quarantined_gpu_submissions, 1);
    }

    #[test]
    fn completion_terminal_result_preserves_timeout_reason() {
        let backend = VulkanBackend::new(test_config());
        let handle = {
            let mut inner = backend.inner.lock();
            let handle = allocate_handle(
                &mut inner,
                VulkanWorkloadClass::SceneComposition,
                VulkanExecutionPath::Vulkan,
                None,
            );
            insert_terminal_result(
                &mut inner,
                handle_key(handle),
                VulkanTerminalRecord {
                    status: VulkanPollStatus::TimedOut,
                    fallback_reason: VulkanFallbackReason::Timeout,
                    completed_at: Instant::now(),
                },
            );
            handle
        };
        assert_eq!(backend.poll_completion(handle), VulkanPollStatus::TimedOut);
    }

    #[tokio::test]
    async fn completion_requires_zeroize_counts_cleared_host_metadata_on_fallback() {
        let backend = VulkanBackend::new(test_config());
        backend.initialize();
        let mut request =
            submission(VulkanWorkloadClass::AuditScan, 8, Some(vec![0xDEAD_BEEF, 0xA5A5_5A5A]));
        request.requires_zeroize = true;
        let result = backend.wait_for_completion(backend.submit_batch(request)).await;
        assert_eq!(result.path, VulkanExecutionPath::CpuFallback);
        assert_eq!(backend.metrics().zeroized_host_bytes, 8);
    }

    #[tokio::test]
    async fn completion_cpu_fallbacks_do_not_accumulate_submission_state() {
        let backend = VulkanBackend::new(test_config());
        backend.initialize();
        for _ in 0..(MAX_PENDING_GPU_SUBMISSIONS * 4) {
            let handle =
                backend.submit_batch(submission(VulkanWorkloadClass::AuditScan, 4, Some(vec![1])));
            let result = backend.wait_for_completion(handle).await;
            assert_eq!(result.path, VulkanExecutionPath::CpuFallback);
        }
        let metrics = backend.metrics();
        assert_eq!(metrics.pending_gpu_submissions, 0);
        assert_eq!(metrics.quarantined_gpu_submissions, 0);
    }
}
