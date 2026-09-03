//! Taking a filled dmabuf back from the compositor.
//!
//! Split from `dmabuf` at the line gate. Memory shared across processes
//! belongs, in Vulkan's terms, to a foreign queue family while the other side
//! has it, and this is the barrier that takes it back.

use ash::vk;
use wgpu::hal::api::Vulkan;

use super::dmabuf::DmabufImage;

/// Takes the image back from whoever wrote it.
///
/// Memory shared across processes belongs, in Vulkan's terms, to a foreign
/// queue family while the other side has it. Reading it without acquiring it
/// first works on every driver tested and is wrong on all of them: the
/// barrier is what orders the compositor's last write before this engine's
/// first read. Submitted on wgpu's own queue, between its submissions, from
/// the thread that owns them.
pub fn acquire(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &DmabufImage,
) -> Result<(), String> {
    let hal_device = unsafe { device.as_hal::<Vulkan>() }.ok_or("the device is not Vulkan")?;
    // Held for the duration so nothing else submits on the queue meanwhile;
    // the raw queue itself is the device's.
    let _hal_queue = unsafe { queue.as_hal::<Vulkan>() }.ok_or("the queue is not Vulkan")?;
    let raw = hal_device.raw_device();
    let raw_queue = hal_device.raw_queue();
    let family = hal_device.queue_family_index();
    let pool = unsafe {
        raw.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT),
            None,
        )
    }
    .map_err(|error| format!("could not create a command pool: {error}"))?;
    let result = (|| {
        let buffer = unsafe {
            raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|error| format!("could not allocate a command buffer: {error}"))?[0];
        unsafe {
            raw.begin_command_buffer(
                buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        }
        .map_err(|error| format!("could not begin a command buffer: {error}"))?;
        let barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            .dst_queue_family_index(family)
            .image(image.raw)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );
        unsafe {
            raw.cmd_pipeline_barrier(
                buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
            raw.end_command_buffer(buffer)
        }
        .map_err(|error| format!("could not end a command buffer: {error}"))?;
        let fence = unsafe { raw.create_fence(&vk::FenceCreateInfo::default(), None) }
            .map_err(|error| format!("could not create a fence: {error}"))?;
        let submitted = unsafe {
            raw.queue_submit(
                raw_queue,
                &[vk::SubmitInfo::default().command_buffers(&[buffer])],
                fence,
            )
        };
        let waited =
            submitted.and_then(|()| unsafe { raw.wait_for_fences(&[fence], true, u64::MAX) });
        unsafe { raw.destroy_fence(fence, None) };
        waited.map_err(|error| format!("could not acquire the capture: {error}"))
    })();
    unsafe { raw.destroy_command_pool(pool, None) };
    result
}
