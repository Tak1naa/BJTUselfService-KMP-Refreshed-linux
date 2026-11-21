//! Compact GBK decoder (`23 KiB` RLE table) for legacy `piGaiDiv` attachment filenames.

const TABLE: &[u8] = include_bytes!("gbk_table.bin");
const SLOTS: usize = 126 * 190;

fn lookup_pair(b1: u8, b2: u8) -> Option<char> {
    if !(0x81..=0xFE).contains(&b1) || !(0x40..=0xFE).contains(&b2) || b2 == 0x7F {
        return None;
    }
    let col = usize::from(b2 - 0x40) - usize::from(b2 > 0x7F);
    let target = usize::from(b1 - 0x81) * 190 + col;
    let mut i = 0;
    let mut pos = 0;
    let mut prev = 0u16;
    while i < TABLE.len() {
        let tag = TABLE[i];
        i += 1;
        match tag {
            0 => {
                let val = u16::from_le_bytes([TABLE[i], TABLE[i + 1]]);
                i += 2;
                if pos == target {
                    return char::from_u32(u32::from(val));
                }
                prev = val;
                pos += 1;
            }
            1..=0x7F => {
                let count = usize::from(tag);
                if target < pos + count {
                    let val = prev.wrapping_add((target - pos + 1) as u16);
                    return char::from_u32(u32::from(val));
                }
                prev = prev.wrapping_add(count as u16);
                pos += count;
            }
            0x80..=0xBF => {
                let count = usize::from(tag - 0x80) + 1;
                if target < pos + count {
                    return Some('\u{FFFD}');
                }
                pos += count;
            }
            0xC0..=0xFF => {
                let val = prev.wrapping_add(u16::from(tag - 0xC0) + 2);
                if pos == target {
                    return char::from_u32(u32::from(val));
                }
                prev = val;
                pos += 1;
            }
        }
    }
    let _ = SLOTS;
    None
}

pub(crate) fn decode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b1 = bytes[i];
        if b1 < 0x80 {
            out.push(b1 as char);
            i += 1;
        } else if b1 == 0x80 {
            out.push('€');
            i += 1;
        } else if i + 1 < bytes.len()
            && let Some(ch) = lookup_pair(b1, bytes[i + 1])
        {
            out.push(ch);
            i += 2;
        } else {
            out.push('\u{FFFD}');
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_gbk_chinese_filenames() {
        // "实验报告" in GBK: CA B5 D1 E9 B1 A8 B8 E6
        let gbk = b"<a href='a.pdf'>\xca\xb5\xd1\xe9\xb1\xa8\xb8\xe6.pdf</a>";
        assert_eq!(decode(gbk), "<a href='a.pdf'>实验报告.pdf</a>");
    }
}
