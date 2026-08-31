use super::volume::*;

#[test]
fn volume_pod_round_trips() {
    let pod = volume_pod(&[0.25, 0.75], true);
    let parsed = unsafe { parse_volume(pod.as_ptr().cast()) }.unwrap();
    assert_eq!(parsed.channels, vec![0.25, 0.75]);
    assert!(parsed.muted);
    assert_eq!(parsed.average(), 0.5);
}

#[test]
fn volume_pod_uses_eight_byte_alignment() {
    assert_eq!(align(9), 16);
    assert_eq!(align(16), 16);
}
