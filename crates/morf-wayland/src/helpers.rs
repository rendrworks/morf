use rustix::fs::{MemfdFlags, memfd_create};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use wayland_client::protocol::wl_output;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

/// Converts a logical size to the physical pixels a surface needs for it.
///
/// The scale is in 120ths, which is how the fractional-scale protocol states
/// it, and the result rounds up: a surface one pixel short of its logical size
/// shows a seam, and one pixel over does not.
///
/// The clamps are the whole reason this is shared rather than written where it
/// is needed. There were three copies of this arithmetic and they disagreed on
/// every edge: what a zero scale means, what a zero size means, and whether a
/// surface may end up zero pixels across. A zero-sized buffer is a protocol
/// error, so nothing here is allowed to produce one.
pub fn physical_size(logical: (u32, u32), scale_120: u32) -> (u32, u32) {
    let scale = scale_120.max(1) as u64;
    (
        ((logical.0 as u64 * scale).div_ceil(120)).max(1) as u32,
        ((logical.1 as u64 * scale).div_ceil(120)).max(1) as u32,
    )
}

pub(crate) fn output_transform_name(transform: wl_output::Transform) -> &'static str {
    match transform {
        wl_output::Transform::Normal => "normal",
        wl_output::Transform::_90 => "90",
        wl_output::Transform::_180 => "180",
        wl_output::Transform::_270 => "270",
        wl_output::Transform::Flipped => "flipped",
        wl_output::Transform::Flipped90 => "flipped_90",
        wl_output::Transform::Flipped180 => "flipped_180",
        wl_output::Transform::Flipped270 => "flipped_270",
        _ => "unknown",
    }
}

pub(crate) fn default_keymap() -> Option<String> {
    let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
    xkbcommon::xkb::Keymap::new_from_names(
        &context,
        "",
        "pc105",
        "us",
        "",
        None,
        xkbcommon::xkb::COMPILE_NO_FLAGS,
    )
    .map(|keymap| keymap.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1))
}

pub(crate) fn install_virtual_keymap(
    keyboard: &ZwpVirtualKeyboardV1,
    keymap: &str,
) -> std::io::Result<File> {
    let mut bytes = keymap.as_bytes().to_vec();
    if !bytes.ends_with(&[0]) {
        bytes.push(0);
    }
    let fd = memfd_create("morf-keymap", MemfdFlags::CLOEXEC)?;
    let mut file = File::from(fd);
    file.write_all(&bytes)?;
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    keyboard.keymap(1, file.as_fd(), bytes.len() as u32);
    Ok(file)
}
