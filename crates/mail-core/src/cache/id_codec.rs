//! Bit-packed external ID encoding for per-account split SQLite caches.
//!
//! External IDs (`folder_id`, `message_id`, `attachment_id`) remain `u32` across
//! `hylki-mail-core` and GPUI UI state:
//! - top 8 bits (`24..32`): persisted account slot `1..=254` in `global.db`
//! - bottom 24 bits (`0..24`): local SQLite rowid `1..=16_777_215` inside that account's `mail.db`
//!
//! Special sentinel IDs such as `0` (`UNIFIED_INBOX_FOLDER_ID`) are never encoded.

pub const MAX_SLOT: u8 = 254;
pub const MAX_LOCAL_ID: u32 = 0x00FF_FFFF;

/// Encode `(slot, local_id)` into a globally unique external `u32` ID.
#[inline]
pub fn encode_scoped_id(slot: u8, local_id: u32) -> Result<u32, String> {
    if slot == 0 || slot > MAX_SLOT {
        return Err(format!("账号槽位越界 ({slot})，有效范围为 1..={MAX_SLOT}"));
    }
    if local_id == 0 || local_id > MAX_LOCAL_ID {
        return Err(format!(
            "单账号本地行 ID 越界 ({local_id})，有效范围为 1..={MAX_LOCAL_ID}"
        ));
    }
    Ok(((slot as u32) << 24) | local_id)
}

/// Decode a globally unique external `u32` ID back into `(slot, local_id)`.
#[inline]
pub fn decode_scoped_id(external_id: u32) -> Option<(u8, u32)> {
    let slot = (external_id >> 24) as u8;
    let local_id = external_id & MAX_LOCAL_ID;
    if slot == 0 || slot > MAX_SLOT || local_id == 0 {
        None
    } else {
        Some((slot, local_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_id_roundtrip_and_bounds() {
        let ext = encode_scoped_id(1, 42).unwrap();
        assert_eq!(decode_scoped_id(ext), Some((1, 42)));

        let ext_max = encode_scoped_id(MAX_SLOT, MAX_LOCAL_ID).unwrap();
        assert_eq!(decode_scoped_id(ext_max), Some((MAX_SLOT, MAX_LOCAL_ID)));

        assert!(encode_scoped_id(0, 1).is_err());
        assert!(encode_scoped_id(255, 1).is_err());
        assert!(encode_scoped_id(1, 0).is_err());
        assert!(encode_scoped_id(1, MAX_LOCAL_ID + 1).is_err());

        assert_eq!(decode_scoped_id(0), None);
        assert_eq!(decode_scoped_id(1), None);
        assert_eq!(decode_scoped_id((255u32 << 24) | 1), None);
        assert_eq!(decode_scoped_id(1u32 << 24), None);
    }
}
