//! The small helpers several modules share (src/util.rs).
#[allow(dead_code)] // only the date and hash helpers are tested here; the rest is tested where it is used
#[path = "../src/util.rs"]
mod util;
use util::{fnv1a, iso};

#[test]
fn dates_come_out_right() {
    assert_eq!(iso(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(iso(1_000_000_000), "2001-09-09T01:46:40Z");
    assert_eq!(iso(1_793_041_603), "2026-10-26T19:06:43Z");
}

/// Files are found by this hash: the values below are what the two copies of it gave before they were merged.
#[test]
fn the_hash_gives_the_values_it_always_gave() {
    for (s, h) in [
        ("", 0xcbf2_9ce4_8422_2325),
        ("a", 0xaf63_dc4c_8601_ec8c),
        ("foobar", 0x8594_4171_f739_67e8),
        ("/srv/work/app", 0xc1b1_3586_c416_5121),
        ("/tmp/proj with space/ü", 0xff2d_7817_9ac5_daa7),
    ] {
        assert_eq!(fnv1a(s.as_bytes()), h, "{s}");
    }
}
