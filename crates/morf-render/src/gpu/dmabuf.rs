//! A texture the compositor can draw into, without a copy.
//!
//! A screen capture used to be shared memory: the compositor rendered the
//! frame on the GPU, copied it out to a `wl_shm` buffer, and this engine
//! uploaded it straight back to the GPU to draw it. Twice across the bus for
//! pixels that never needed to leave. This is the other way: an image is
//! created here, exported as a dmabuf, handed to the compositor as the buffer
//! to capture *into*, and drawn from the same memory when the frame is ready.
//!
//! wgpu has no word for any of that, so this speaks Vulkan directly for the
//! three things it needs -- exporting an image, asking what it looks like in
//! memory, and acquiring it back from a foreign queue -- and hands the result
//! to wgpu as a texture it did not create but will happily draw. Everything
//! is behind the extensions being present; on a device without them the
//! shared-memory path is what runs, as it always did.

use std::ffi::CStr;
use std::os::fd::{FromRawFd, OwnedFd};

use ash::vk;
use wgpu::hal::api::Vulkan;

/// `DRM_FORMAT_XRGB8888`: little-endian bytes blue, green, red, padding.
pub const FOURCC_XRGB8888: u32 = 0x3432_5258;
/// `DRM_FORMAT_ARGB8888`: the same with alpha where the padding was.
pub const FOURCC_ARGB8888: u32 = 0x3432_5241;
/// `DRM_FORMAT_MOD_LINEAR`: rows in order, no tiling.
pub const MODIFIER_LINEAR: u64 = 0;

/// The device extensions dmabuf export needs, and whether each is required.
///
/// The first four are what makes an exportable image possible at all. The
/// fifth says which DRM node the device is, so a compositor's offer can be
/// checked against it: a buffer allocated on one GPU and captured into by
/// another is a buffer full of noise. Optional, because a device without it
/// can still export -- it just cannot prove it is the right one.
pub(crate) const EXTENSIONS: [(&CStr, bool); 5] = [
    (ash::khr::external_memory_fd::NAME, true),
    (ash::ext::external_memory_dma_buf::NAME, true),
    (ash::ext::image_drm_format_modifier::NAME, true),
    (ash::ext::queue_family_foreign::NAME, true),
    (ash::ext::physical_device_drm::NAME, false),
];

/// What the device turned out to be able to do.
#[derive(Clone, Debug)]
pub struct DmabufSupport {
    /// The render node this device is, as major and minor, when it said.
    pub render_node: Option<(u32, u32)>,
    /// The queue family every wgpu command runs on.
    pub queue_family: u32,
}

/// One plane of an exported image: the file descriptor and where the pixels
/// sit behind it.
#[derive(Debug)]
pub struct DmabufPlane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

/// An image exported as a dmabuf, and the wgpu texture that reads it.
///
/// The Vulkan image and its memory belong to the texture: wgpu destroys them
/// through the drop callback it was given, after every command that touched
/// the texture has finished. Nothing here needs a destructor of its own.
pub struct DmabufImage {
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub modifier: u64,
    pub plane: DmabufPlane,
    pub texture: wgpu::Texture,
    pub(crate) raw: vk::Image,
}

/// The extensions this physical device offers, out of [`EXTENSIONS`].
///
/// Returns them only when every required one is there: a device with export
/// but no modifiers cannot say what it exported, and a partial set is no set.
pub(crate) fn supported_extensions(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
) -> Option<Vec<&'static CStr>> {
    let available = unsafe { instance.enumerate_device_extension_properties(physical) }.ok()?;
    let has = |wanted: &CStr| {
        available.iter().any(|extension| {
            extension
                .extension_name_as_c_str()
                .is_ok_and(|name| name == wanted)
        })
    };
    let mut enabled = Vec::new();
    for (name, required) in EXTENSIONS {
        if has(name) {
            enabled.push(name);
        } else if required {
            return None;
        }
    }
    Some(enabled)
}

/// Which DRM render node a physical device is, from `VK_EXT_physical_device_drm`.
pub(crate) fn render_node(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
) -> Option<(u32, u32)> {
    let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    (drm.has_render == vk::TRUE).then_some((drm.render_major as u32, drm.render_minor as u32))
}

/// Splits a Linux `dev_t` the way the kernel packs it.
///
/// Not `major = dev >> 8`: the modern encoding scatters both numbers across
/// the word so that old programs keep working, and a device number read the
/// old way names the wrong node on any machine with more than a few.
pub fn split_dev_t(dev: u64) -> (u32, u32) {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major as u32, minor as u32)
}

/// The Vulkan format a DRM fourcc is drawn as, for the two a capture offers.
fn vulkan_format(fourcc: u32) -> Option<vk::Format> {
    match fourcc {
        FOURCC_XRGB8888 | FOURCC_ARGB8888 => Some(vk::Format::B8G8R8A8_UNORM),
        _ => None,
    }
}

/// The modifiers this device can export `format` with, single-plane only.
///
/// A modifier with a second memory plane -- Intel's compression modifiers,
/// for one -- would need a second file descriptor, and a capture protocol
/// that takes one buffer with one plane per format cannot carry it. Those are
/// left out rather than half-handled; what remains still includes the tiled
/// layouts, which is where the speed is.
fn exportable_modifiers(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    format: vk::Format,
) -> Vec<u64> {
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
    let mut properties = vk::FormatProperties2::default().push_next(&mut list);
    unsafe { instance.get_physical_device_format_properties2(physical, format, &mut properties) };
    let count = list.drm_format_modifier_count as usize;
    if count == 0 {
        return Vec::new();
    }
    let mut entries = vec![vk::DrmFormatModifierPropertiesEXT::default(); count];
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
        .drm_format_modifier_properties(&mut entries);
    let mut properties = vk::FormatProperties2::default().push_next(&mut list);
    unsafe { instance.get_physical_device_format_properties2(physical, format, &mut properties) };
    let wanted = vk::FormatFeatureFlags::SAMPLED_IMAGE
        | vk::FormatFeatureFlags::TRANSFER_DST
        | vk::FormatFeatureFlags::TRANSFER_SRC;
    entries
        .iter()
        .filter(|entry| entry.drm_format_modifier_plane_count == 1)
        .filter(|entry| entry.drm_format_modifier_tiling_features.contains(wanted))
        .map(|entry| entry.drm_format_modifier)
        .collect()
}

/// The device's exportable modifiers for a fourcc, through wgpu's device.
pub(crate) fn modifiers_for(device: &wgpu::Device, fourcc: u32) -> Vec<u64> {
    let Some(format) = vulkan_format(fourcc) else {
        return Vec::new();
    };
    let Some(hal) = (unsafe { device.as_hal::<Vulkan>() }) else {
        return Vec::new();
    };
    exportable_modifiers(
        hal.shared_instance().raw_instance(),
        hal.raw_physical_device(),
        format,
    )
}

/// Creates an image the compositor can capture into, exported as a dmabuf.
///
/// `offered` is the compositor's modifier list for `fourcc`; the first of the
/// device's own that the compositor also accepts is used, in the device's
/// order, because the device lists what it draws fastest first.
pub fn export(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    fourcc: u32,
    offered: &[u64],
) -> Result<DmabufImage, String> {
    let format =
        vulkan_format(fourcc).ok_or_else(|| format!("no Vulkan format for fourcc {fourcc:#x}"))?;
    let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or("the device is not Vulkan")?;
    let raw = hal.raw_device();
    let instance = hal.shared_instance().raw_instance();
    let physical = hal.raw_physical_device();
    let queue_family = hal.queue_family_index();

    let candidates = exportable_modifiers(instance, physical, format);
    let modifiers = candidates
        .iter()
        .copied()
        .filter(|modifier| offered.contains(modifier))
        .collect::<Vec<_>>();
    if modifiers.is_empty() {
        return Err("the compositor and the GPU agree on no modifier".to_owned());
    }

    // The image: tiled by whichever modifier the driver picks from the list,
    // and marked from birth as one that will be exported.
    let mut modifier_list =
        vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(
            vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut modifier_list)
        .push_next(&mut external);
    let image = unsafe { raw.create_image(&image_info, None) }
        .map_err(|error| format!("could not create an exportable image: {error}"))?;

    // Its memory: dedicated, because an exported allocation is one the driver
    // wants to own outright, and exportable as a dmabuf.
    let mut dedicated_requirements = vk::MemoryDedicatedRequirements::default();
    let mut requirements =
        vk::MemoryRequirements2::default().push_next(&mut dedicated_requirements);
    unsafe {
        raw.get_image_memory_requirements2(
            &vk::ImageMemoryRequirementsInfo2::default().image(image),
            &mut requirements,
        )
    };
    let requirements = requirements.memory_requirements;
    let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical) };
    let memory_type = (0..memory_properties.memory_type_count)
        .find(|&index| {
            requirements.memory_type_bits & (1 << index) != 0
                && memory_properties.memory_types[index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .or_else(|| {
            (0..memory_properties.memory_type_count)
                .find(|&index| requirements.memory_type_bits & (1 << index) != 0)
        });
    let Some(memory_type) = memory_type else {
        unsafe { raw.destroy_image(image, None) };
        return Err("no memory type can back an exportable image".to_owned());
    };
    let mut export_info = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocate_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type)
        .push_next(&mut export_info)
        .push_next(&mut dedicated);
    let memory = match unsafe { raw.allocate_memory(&allocate_info, None) } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe { raw.destroy_image(image, None) };
            return Err(format!("could not allocate exportable memory: {error}"));
        }
    };
    if let Err(error) = unsafe { raw.bind_image_memory(image, memory, 0) } {
        unsafe {
            raw.free_memory(memory, None);
            raw.destroy_image(image, None);
        }
        return Err(format!("could not bind exportable memory: {error}"));
    }

    // What it looks like from outside: which modifier the driver chose, and
    // the plane's offset and stride, which is what a wl_buffer is made of.
    let modifier_device = ash::ext::image_drm_format_modifier::Device::new(instance, raw);
    let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
    let modifier = match unsafe {
        modifier_device.get_image_drm_format_modifier_properties(image, &mut chosen)
    } {
        Ok(()) => chosen.drm_format_modifier,
        Err(error) => {
            unsafe {
                raw.free_memory(memory, None);
                raw.destroy_image(image, None);
            }
            return Err(format!(
                "the driver would not say which modifier it used: {error}"
            ));
        }
    };
    let layout = unsafe {
        raw.get_image_subresource_layout(
            image,
            vk::ImageSubresource::default()
                .aspect_mask(vk::ImageAspectFlags::MEMORY_PLANE_0_EXT)
                .mip_level(0)
                .array_layer(0),
        )
    };
    let fd_device = ash::khr::external_memory_fd::Device::new(instance, raw);
    let fd = match unsafe {
        fd_device.get_memory_fd(
            &vk::MemoryGetFdInfoKHR::default()
                .memory(memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
        )
    } {
        Ok(fd) => unsafe { OwnedFd::from_raw_fd(fd) },
        Err(error) => {
            unsafe {
                raw.free_memory(memory, None);
                raw.destroy_image(image, None);
            }
            return Err(format!("could not export the image as a dmabuf: {error}"));
        }
    };

    // And as a wgpu texture. The Vulkan objects go to wgpu with a callback
    // that destroys them, so their life is the texture's and ends after the
    // last command that read it. `STORAGE_READ_ONLY` as the starting state is
    // deliberate: it is the one that maps to `GENERAL`, the layout an image
    // filled from outside is in, and the only old layout a first barrier may
    // name without being allowed to throw the contents away.
    let raw_for_drop = raw.clone();
    let hal_texture = unsafe {
        hal.texture_from_raw(
            image,
            &wgpu::hal::TextureDescriptor {
                label: Some("morf dmabuf capture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::wgt::TextureUses::RESOURCE
                    | wgpu::wgt::TextureUses::COPY_DST
                    | wgpu::wgt::TextureUses::COPY_SRC,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: vec![wgpu::TextureFormat::Bgra8UnormSrgb],
            },
            // Called by wgpu after the last command that used the texture
            // has completed, and the handles are ours: inside the enclosing
            // unsafe block, which is what makes the calls permitted.
            Some(Box::new(move || {
                raw_for_drop.free_memory(memory, None);
                raw_for_drop.destroy_image(image, None);
            })),
            wgpu::hal::vulkan::TextureMemory::External,
        )
    };
    drop(hal);
    let texture = unsafe {
        device.create_texture_from_hal::<Vulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("morf dmabuf capture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[wgpu::TextureFormat::Bgra8UnormSrgb],
            },
            wgpu::wgt::TextureUses::STORAGE_READ_ONLY,
        )
    };
    let _ = queue_family;
    Ok(DmabufImage {
        width,
        height,
        fourcc,
        modifier,
        plane: DmabufPlane {
            fd,
            offset: layout.offset as u32,
            stride: layout.row_pitch as u32,
        },
        texture,
        raw: image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_t_splits_the_way_the_kernel_packs_it() {
        // /dev/dri/renderD128 is 226:128. Packed old-style that is 0xe280;
        // packed the modern way the minor's high byte moves out past bit 20.
        assert_eq!(split_dev_t(0xe280), (226, 128));
        assert_eq!(split_dev_t(0xe281), (226, 129));
        // A minor above 255: 226:300 packs as (300 & 0xff) | (300 >> 8 << 20)
        // with the major in bits 8..20.
        let packed = (226u64 << 8) | (300 & 0xff) | ((300u64 >> 8) << 20);
        assert_eq!(split_dev_t(packed), (226, 300));
    }

    #[test]
    fn only_the_two_capture_formats_have_a_vulkan_face() {
        assert_eq!(
            vulkan_format(FOURCC_XRGB8888),
            Some(vk::Format::B8G8R8A8_UNORM)
        );
        assert_eq!(
            vulkan_format(FOURCC_ARGB8888),
            Some(vk::Format::B8G8R8A8_UNORM)
        );
        assert_eq!(
            vulkan_format(0x3231_3652),
            None,
            "RG16 is not a capture format"
        );
    }
}
