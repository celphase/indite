use std::ffi::CStr;

use openxr::{
    Entry, Instance,
    raw::DebugUtilsEXT,
    sys::{
        Bool32, DebugUtilsMessageSeverityFlagsEXT, DebugUtilsMessageTypeFlagsEXT,
        DebugUtilsMessengerCallbackDataEXT, DebugUtilsMessengerCreateInfoEXT,
        DebugUtilsMessengerEXT, FALSE,
    },
};

pub struct DebugUtils {
    // Guard over these so they're kept alive longer than DebugUtils
    _xr_entry: Entry,
    _xr_instance: Instance,

    _debug_utils: DebugUtilsEXT,
    debug_messenger: DebugUtilsMessengerEXT,
}

impl DebugUtils {
    pub fn new(xr_entry: &Entry, xr_instance: &Instance) -> Option<Self> {
        let debug_utils = unsafe { DebugUtilsEXT::load(xr_entry, xr_instance.as_raw()).ok()? };

        let mut debug_messenger = DebugUtilsMessengerEXT::default();

        unsafe {
            let debug_info = DebugUtilsMessengerCreateInfoEXT {
                ty: DebugUtilsMessengerCreateInfoEXT::TYPE,
                next: std::ptr::null(),
                message_severities: DebugUtilsMessageSeverityFlagsEXT::INFO
                    | DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | DebugUtilsMessageSeverityFlagsEXT::ERROR,
                message_types: DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | DebugUtilsMessageTypeFlagsEXT::PERFORMANCE
                    | DebugUtilsMessageTypeFlagsEXT::CONFORMANCE,
                user_callback: Some(handle_validation_message),
                user_data: std::ptr::null_mut(),
            };
            (debug_utils.create_debug_utils_messenger)(
                xr_instance.as_raw(),
                &debug_info,
                &mut debug_messenger,
            );
        }

        Some(Self {
            _xr_entry: xr_entry.clone(),
            _xr_instance: xr_instance.clone(),

            _debug_utils: debug_utils,
            debug_messenger,
        })
    }
}

impl Drop for DebugUtils {
    fn drop(&mut self) {
        unsafe {
            (self._debug_utils.destroy_debug_utils_messenger)(self.debug_messenger);
        }
    }
}

unsafe extern "system" fn handle_validation_message(
    severity: DebugUtilsMessageSeverityFlagsEXT,
    ty: DebugUtilsMessageTypeFlagsEXT,
    callback: *const DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut std::ffi::c_void,
) -> Bool32 {
    let ty = match ty {
        DebugUtilsMessageTypeFlagsEXT::GENERAL => "general",
        DebugUtilsMessageTypeFlagsEXT::VALIDATION => "validation",
        DebugUtilsMessageTypeFlagsEXT::PERFORMANCE => "performance",
        DebugUtilsMessageTypeFlagsEXT::CONFORMANCE => "conformance",
        _ => "unknown",
    };
    let severity = match severity {
        DebugUtilsMessageSeverityFlagsEXT::VERBOSE => "verbose",
        DebugUtilsMessageSeverityFlagsEXT::INFO => "info",
        DebugUtilsMessageSeverityFlagsEXT::WARNING => "warning",
        DebugUtilsMessageSeverityFlagsEXT::ERROR => "error",
        _ => "unknown",
    };

    let message = unsafe { CStr::from_ptr((*callback).message) };

    println!("openxr validation ({} {}): {:?}", ty, severity, message);

    FALSE
}
