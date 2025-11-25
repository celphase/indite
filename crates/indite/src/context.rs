use std::ffi::CStr;

use anyhow::{Context, Error, bail};
use ash::vk::{self, Handle};
use wgpu::{
    Device, DeviceDescriptor, ExperimentalFeatures, Features, Instance, InstanceFlags, Limits,
    MemoryBudgetThresholds, MemoryHints, Queue, Trace,
    hal::{Api, ExposedAdapter, api::Vulkan},
};

pub fn create_instance(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
) -> Result<Instance, Error> {
    // Vulkan 1.1 guarantees multiview support
    // It seems WGPU internally is promoting timeline_semaphore, using the core version. This isn't
    //  actually core in 1.1 however, so use 1.2 for now.
    let vk_target_version = vk::make_api_version(0, 1, 2, 0);
    let vk_target_version_xr = openxr::Version::new(1, 2, 0);

    // The `graphics_requirement` call is a required call. If you don't do it before anything else,
    // things break! No, really. If your runtime doesn't break if you don't call this, good for you,
    // but mine did! It was a gaint pain to debug! So don't remove this call!
    let reqs = xr_instance.graphics_requirements::<openxr::Vulkan>(xr_system)?;

    if vk_target_version_xr < reqs.min_api_version_supported
        || vk_target_version_xr.major() > reqs.max_api_version_supported.major()
    {
        bail!(
            "openxr runtime requires vulkan version > {}, <= {}",
            reqs.min_api_version_supported,
            reqs.max_api_version_supported.major()
        );
    }

    let vk_entry = unsafe { ash::Entry::load()? };

    let (vk_instance, extensions, flags) =
        unsafe { create_vk_instance(xr_instance, xr_system, &vk_entry, vk_target_version)? };

    // Create the WPGU instance from the raw instance
    let hal_instance = unsafe {
        <Vulkan as Api>::Instance::from_raw(
            vk_entry.clone(),
            vk_instance.clone(),
            vk_target_version,
            0,
            None,
            extensions,
            flags,
            MemoryBudgetThresholds::default(),
            false,
            None,
        )?
    };

    let instance = unsafe { Instance::from_hal::<Vulkan>(hal_instance) };

    Ok(instance)
}

unsafe fn create_vk_instance(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    vk_entry: &ash::Entry,
    vk_target_version: u32,
) -> Result<(ash::Instance, Vec<&'static CStr>, InstanceFlags), Error> {
    let vk_app_info = vk::ApplicationInfo::default()
        .application_version(0)
        .engine_version(0)
        .api_version(vk_target_version);

    // Fetch extensions needed by WPGU
    let flags = InstanceFlags::empty();
    let extensions =
        <Vulkan as Api>::Instance::desired_extensions(vk_entry, vk_target_version, flags)?;
    let extensions_cchar: Vec<_> = extensions.iter().map(|s| s.as_ptr()).collect();

    let instance_info = vk::InstanceCreateInfo::default()
        .application_info(&vk_app_info)
        .enabled_extension_names(&extensions_cchar);

    // Let OpenXR create the instance
    let get_instance_proc_addr = unsafe {
        std::mem::transmute::<
            ash::vk::PFN_vkGetInstanceProcAddr,
            openxr::sys::platform::VkGetInstanceProcAddr,
        >(vk_entry.static_fn().get_instance_proc_addr)
    };
    let vk_instance = unsafe {
        xr_instance.create_vulkan_instance(
            xr_system,
            get_instance_proc_addr,
            &instance_info as *const _ as *const _,
        )
    };
    let vk_instance = vk_instance
        .context("xr error creating vulkan instance")?
        .map_err(vk::Result::from_raw)
        .context("vulkan error creating vulkan instance")?;

    // Convert to ash instance
    let vk_instance = unsafe {
        ash::Instance::load(
            vk_entry.static_fn(),
            vk::Instance::from_raw(vk_instance as _),
        )
    };

    Ok((vk_instance, extensions, flags))
}

pub fn create_device(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    instance: &Instance,
) -> Result<(Device, Queue), Error> {
    let hal_instance = unsafe { instance.as_hal::<Vulkan>() };
    let hal_instance = hal_instance.context("wgpu instance backend not vulkan")?;
    let shared = hal_instance.shared_instance();

    let vk_physical_device = get_vk_physical_device(
        xr_instance,
        xr_system,
        shared.instance_api_version(),
        shared.raw_instance(),
    )?;

    // Get the WGPU adapter for the picked physical device
    let hal_adapter = hal_instance
        .expose_adapter(vk_physical_device)
        .context("failed to expose adapter")?;

    let features =
        // Required for efficiently rendering both sides
        Features::MULTIVIEW |
        // Required for MSAA rendering, we need a texture that's both an array and has multisample
        Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;

    let (queue_family_index, device_extensions, vk_device) = unsafe {
        create_vk_device(
            xr_instance,
            xr_system,
            shared.entry(),
            shared.raw_instance(),
            vk_physical_device,
            &hal_adapter,
            features,
        )?
    };

    // Get the WPGU open device for the created device
    let memory_hints = MemoryHints::default();
    let hal_device = unsafe {
        hal_adapter.adapter.device_from_raw(
            vk_device.clone(),
            None,
            &device_extensions,
            features,
            &memory_hints,
            queue_family_index,
            0,
        )?
    };

    // Create the WPGU Device handles from all the raw stuff we prepared
    let wgpu_adapter = unsafe { instance.create_adapter_from_hal(hal_adapter) };
    let device_desc = DeviceDescriptor {
        label: Some("vr device"),
        required_features: features,
        required_limits: Limits {
            max_bind_groups: 8,
            max_storage_buffer_binding_size: wgpu_adapter.limits().max_storage_buffer_binding_size,
            max_push_constant_size: 4,
            max_multiview_view_count: 2,
            ..Default::default()
        },
        experimental_features: ExperimentalFeatures::default(),
        memory_hints,
        trace: Trace::default(),
    };
    let (device, queue) = unsafe { wgpu_adapter.create_device_from_hal(hal_device, &device_desc)? };

    Ok((device, queue))
}

fn get_vk_physical_device(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    vk_target_version: u32,
    vk_instance: &ash::Instance,
) -> Result<vk::PhysicalDevice, Error> {
    let vk_physical_device_raw = unsafe {
        xr_instance.vulkan_graphics_device(xr_system, vk_instance.handle().as_raw() as _)
    };
    let vk_physical_device_raw = vk_physical_device_raw
        .ok()
        .context("unable to get physical device, runtime may not be running")?;
    let vk_physical_device = vk::PhysicalDevice::from_raw(vk_physical_device_raw as _);

    let vk_device_properties =
        unsafe { vk_instance.get_physical_device_properties(vk_physical_device) };
    if vk_device_properties.api_version < vk_target_version {
        bail!("vulkan physical device doesn't support version 1.2");
    }

    Ok(vk_physical_device)
}

unsafe fn create_vk_device(
    xr_instance: &openxr::Instance,
    xr_system: openxr::SystemId,
    vk_entry: &ash::Entry,
    vk_instance: &ash::Instance,
    vk_physical_device: vk::PhysicalDevice,
    hal_adapter: &ExposedAdapter<Vulkan>,
    features: Features,
) -> Result<(u32, Vec<&'static CStr>, ash::Device), Error> {
    let queue_family_index = unsafe {
        vk_instance
            .get_physical_device_queue_family_properties(vk_physical_device)
            .into_iter()
            .enumerate()
            .find_map(|(queue_family_index, info)| {
                if info.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                    Some(queue_family_index as u32)
                } else {
                    None
                }
            })
            .context("vulkan device has no graphics queue")?
    };

    // Get the device extensions for the request WPGU features
    let device_extensions = hal_adapter.adapter.required_device_extensions(features);
    let device_extensions_cchar: Vec<_> = device_extensions.iter().map(|s| s.as_ptr()).collect();

    let mut enabled_phd_features = hal_adapter
        .adapter
        .physical_device_features(&device_extensions, features);

    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&[1.0]);
    let queue_infos = [queue_info];
    let mut multiview_features = vk::PhysicalDeviceMultiviewFeatures {
        multiview: vk::TRUE,
        ..Default::default()
    };

    let device_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_infos)
        .enabled_extension_names(&device_extensions_cchar)
        .push_next(&mut multiview_features);
    let device_info = enabled_phd_features.add_to_device_create(device_info);

    let get_instance_proc_addr = unsafe {
        std::mem::transmute::<
            ash::vk::PFN_vkGetInstanceProcAddr,
            openxr::sys::platform::VkGetInstanceProcAddr,
        >(vk_entry.static_fn().get_instance_proc_addr)
    };
    let vk_device = unsafe {
        xr_instance.create_vulkan_device(
            xr_system,
            get_instance_proc_addr,
            vk_physical_device.as_raw() as _,
            &device_info as *const _ as *const _,
        )
    };

    let vk_device = vk_device
        .context("xr error creating vulkan device")?
        .map_err(vk::Result::from_raw)
        .context("vulkan error creating vulkan device")?;

    let vk_device =
        unsafe { ash::Device::load(vk_instance.fp_v1_0(), vk::Device::from_raw(vk_device as _)) };

    Ok((queue_family_index, device_extensions, vk_device))
}
