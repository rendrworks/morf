use libloading::Library;
use std::collections::BTreeMap;
use std::env;
use std::ffi::{CStr, c_char};
use std::path::PathBuf;
use std::ptr;

use super::ffi::*;
use super::*;

pub(crate) unsafe fn dict(props: *const SpaDict) -> BTreeMap<String, String> {
    let props = unsafe { &*props };
    let mut result = BTreeMap::new();
    for index in 0..props.n_items as usize {
        let item = unsafe { &*props.items.add(index) };
        if let (Some(key), Some(value)) = (c_string(item.key), c_string(item.value)) {
            result.insert(key, value);
        }
    }
    result
}

pub(crate) fn c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

pub(crate) fn load_library() -> Result<Library, PipeWireError> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("MOLD_PIPEWIRE_LIBRARY") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib/libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib64/libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib/x86_64-linux-gnu/libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib/aarch64-linux-gnu/libpipewire-0.3.so.0"),
    ]);
    let mut errors = Vec::new();
    for candidate in candidates {
        match unsafe { Library::new(&candidate) } {
            Ok(library) => return Ok(library),
            Err(error) => errors.push(format!("{}: {error}", candidate.display())),
        }
    }
    Err(PipeWireError(format!(
        "could not load PipeWire ({})",
        errors.join("; ")
    )))
}

pub(crate) fn align(value: usize) -> usize {
    (value + 7) & !7
}

pub(crate) fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_ne_bytes());
}

pub(crate) fn pad(bytes: &mut Vec<u8>) {
    bytes.resize(align(bytes.len()), 0);
}

pub(crate) fn volume_pod(channels: &[f32], muted: bool) -> Vec<u8> {
    let mut properties = Vec::new();
    push_u32(&mut properties, PROP_MUTE);
    push_u32(&mut properties, 0);
    push_u32(&mut properties, 4);
    push_u32(&mut properties, TYPE_BOOL);
    push_u32(&mut properties, u32::from(muted));
    pad(&mut properties);

    push_u32(&mut properties, PROP_CHANNEL_VOLUMES);
    push_u32(&mut properties, 0);
    push_u32(&mut properties, 8 + channels.len() as u32 * 4);
    push_u32(&mut properties, TYPE_ARRAY);
    push_u32(&mut properties, 4);
    push_u32(&mut properties, TYPE_FLOAT);
    for channel in channels {
        push_u32(&mut properties, channel.to_bits());
    }
    pad(&mut properties);

    let mut pod = Vec::new();
    push_u32(&mut pod, 8 + properties.len() as u32);
    push_u32(&mut pod, TYPE_OBJECT);
    push_u32(&mut pod, TYPE_OBJECT_PROPS);
    push_u32(&mut pod, PARAM_PROPS);
    pod.extend(properties);
    pod
}

pub(crate) unsafe fn parse_volume(pod: *const SpaPod) -> Option<PipeWireVolume> {
    let pod = unsafe { &*pod };
    if pod.kind != TYPE_OBJECT || pod.size < 8 {
        return None;
    }
    let base = pod as *const SpaPod as *const u8;
    let end = 8usize.checked_add(pod.size as usize)?;
    let mut offset = 16usize;
    let mut channels = Vec::new();
    let mut muted = false;
    while offset.checked_add(16)? <= end {
        let key = unsafe { ptr::read_unaligned(base.add(offset).cast::<u32>()) };
        let value_size =
            unsafe { ptr::read_unaligned(base.add(offset + 8).cast::<u32>()) } as usize;
        let value_type = unsafe { ptr::read_unaligned(base.add(offset + 12).cast::<u32>()) };
        if offset.checked_add(16)?.checked_add(value_size)? > end {
            return None;
        }
        if key == PROP_MUTE && value_type == TYPE_BOOL && value_size >= 4 {
            muted = unsafe { ptr::read_unaligned(base.add(offset + 16).cast::<u32>()) } != 0;
        } else if key == PROP_VOLUME && value_type == TYPE_FLOAT && value_size >= 4 {
            let bits = unsafe { ptr::read_unaligned(base.add(offset + 16).cast::<u32>()) };
            channels = vec![f32::from_bits(bits)];
        } else if key == PROP_CHANNEL_VOLUMES && value_type == TYPE_ARRAY && value_size >= 8 {
            let child_size =
                unsafe { ptr::read_unaligned(base.add(offset + 16).cast::<u32>()) } as usize;
            let child_type = unsafe { ptr::read_unaligned(base.add(offset + 20).cast::<u32>()) };
            if child_size == 4 && child_type == TYPE_FLOAT {
                channels.clear();
                let count = (value_size - 8) / child_size;
                for index in 0..count {
                    let bits = unsafe {
                        ptr::read_unaligned(base.add(offset + 24 + index * 4).cast::<u32>())
                    };
                    channels.push(f32::from_bits(bits));
                }
            }
        }
        offset = align(offset + 16 + value_size);
    }
    Some(PipeWireVolume { channels, muted })
}
