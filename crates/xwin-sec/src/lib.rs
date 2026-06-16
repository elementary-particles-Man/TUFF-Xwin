pub mod age_assurance_browser_surface_boundary;
pub mod browser;
pub mod browser_surface_boundary;
pub mod capability;
pub mod client;
pub mod decision;
pub mod policy;

pub use age_assurance_browser_surface_boundary::{
    AgeAssuranceBrowserOperatorOverrides, AgeAssuranceBrowserSurfaceBoundaryAction,
    AgeAssuranceBrowserSurfaceBoundaryContext, AgeAssuranceBrowserSurfaceBoundaryDecision,
    AgeAssuranceBrowserSurfaceBoundaryDecisionState, AgeAssuranceBrowserSurfaceBoundaryFinding,
    AgeAssuranceBrowserSurfaceState, evaluate_age_assurance_browser_surface_boundary,
};
pub use browser::{browser_hostile_client, browser_hostile_policy_note};
pub use browser_surface_boundary::{
    BrowserClipboardPolicy, BrowserExtensionOrNativePolicy, BrowserFileBoundaryPolicy,
    BrowserGpuBoundaryPolicy, BrowserInputBoundaryPolicy, BrowserOperatorOverrides,
    BrowserRuntimeFamily, BrowserRuntimePosture, BrowserSurfaceBoundaryAction,
    BrowserSurfaceBoundaryContext, BrowserSurfaceBoundaryDecision,
    BrowserSurfaceBoundaryDecisionState, BrowserSurfaceBoundaryFinding, BrowserSurfaceState,
    BrowserWindowRole, evaluate_browser_surface_boundary,
};
pub use capability::{CapabilityGrant, GrantLifetime, GrantScope, XwinCapability};
pub use client::{AppId, ClientId, ClientKind, ClientProfile, ClientTrust, SurfaceId};
pub use decision::{DecisionReason, PolicyContext, SecurityDecision};
pub use policy::{BrowserSecurityPolicy, SecurityPolicy};
pub mod privileged_ai_surface_boundary;
pub use privileged_ai_surface_boundary::*;
pub mod developer_tool_surface_boundary;
pub use developer_tool_surface_boundary::*;
