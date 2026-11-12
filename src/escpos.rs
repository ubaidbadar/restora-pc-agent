/// ESC/POS bytes for a thermal receipt printer. Text is plain ASCII: anything else is swapped or printed as `?`.
pub struct Escpos {
    pub bytes: Vec<u8>,
    /// Characters per line: 48 on an 80 mm roll, 32 on 58 mm.
    pub width: usize,
    /// The current character size; a double-size line holds half the characters.
    scale: usize,
}

impl Escpos {
    pub fn new(width: usize) -> Self {
        Self {
            bytes: vec![0x1b, b'@'],
            width,
            scale: 1,
        }
    }

    /// Prints `text` and ends the line.
    pub fn line(&mut self, text: &str) {
        self.bytes.extend(ascii(text));
        self.bytes.push(b'\n');
    }

    /// `left` at the start of the line, `right` at the end; `left` is cut short when they do not fit.
    pub fn pair(&mut self, left: &str, right: &str) {
        let left = ascii(left);
        let right = ascii(right);
        let width = self.width / self.scale;
        if width == 0 {
            self.bytes.push(b'\n');
            return;
        }
        // Keep the end of long totals, including the monetary digits.
        let right = &right[right.len().saturating_sub(width)..];
        let room = width.saturating_sub(right.len() + 1);
        let left = &left[..left.len().min(room)];
        self.bytes.extend(left);
        self.bytes
            .extend(std::iter::repeat_n(b' ', width - left.len() - right.len()));
        self.bytes.extend(right);
        self.bytes.push(b'\n');
    }

    pub fn rule(&mut self, mark: char) {
        self.line(&mark.to_string().repeat(self.width / self.scale));
    }

    pub fn align(&mut self, side: Side) {
        self.bytes.extend([0x1b, b'a', side as u8]);
    }

    pub fn bold(&mut self, on: bool) {
        self.bytes.extend([0x1b, b'E', on as u8]);
    }

    /// White on black, how a slip marks what not to make.
    pub fn invert(&mut self, on: bool) {
        self.bytes.extend([0x1d, b'B', on as u8]);
    }

    /// Character size, 1 to 4 times the normal width and height.
    pub fn size(&mut self, times: u8) {
        self.scale = times.clamp(1, 4) as usize;
        let n = self.scale as u8 - 1;
        self.bytes.extend([0x1d, b'!', (n << 4) | n]);
    }

    /// Feeds the paper past the cutter and cuts.
    pub fn cut(&mut self) {
        self.bytes.extend([b'\n', b'\n', b'\n', 0x1d, b'V', 66, 0]);
    }
}

#[derive(Clone, Copy)]
pub enum Side {
    Left = 0,
    Center = 1,
}

/// Wraps text to `width` characters per line, breaking at spaces.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    // All characters have been mapped to ASCII, so byte slices are safe here.
    let normalized = ascii_string(text);
    for word in normalized.split_whitespace() {
        let mut remaining = word;
        while !remaining.is_empty() {
            let separator = usize::from(!current.is_empty());
            let available = width.saturating_sub(current.len() + separator);
            if available == 0 {
                lines.push(std::mem::take(&mut current));
                continue;
            }
            if remaining.len() <= available {
                if separator != 0 {
                    current.push(' ');
                }
                current.push_str(remaining);
                break;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                continue;
            }
            let (chunk, tail) = remaining.split_at(available);
            lines.push(chunk.to_string());
            remaining = tail;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn ascii_string(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ' '..='~' => c,
            '×' => 'x',
            '“' | '”' => '"',
            '‘' | '’' => '\'',
            '·' | '–' | '—' | '−' => '-',
            c if c.is_whitespace() => ' ',
            _ => '?',
        })
        .collect()
}

fn ascii(text: &str) -> Vec<u8> {
    ascii_string(text).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_widths_generate_the_expected_rule_lengths() {
        for width in [32, 48] {
            let mut printer = Escpos::new(width);
            printer.rule('-');
            assert_eq!(
                &printer.bytes[2..],
                format!("{}\n", "-".repeat(width)).as_bytes()
            );
        }
    }

    #[test]
    fn double_size_uses_half_the_paper_columns() {
        let mut printer = Escpos::new(32);
        printer.size(2);
        printer.rule('-');
        assert!(
            printer
                .bytes
                .ends_with(format!("{}\n", "-".repeat(16)).as_bytes())
        );
    }

    #[test]
    fn monetary_columns_are_kept_right_aligned() {
        let mut printer = Escpos::new(32);
        printer.pair("Total", "Rs 2,900");
        assert!(
            printer
                .bytes
                .ends_with(b"Total                   Rs 2,900\n")
        );
    }

    #[test]
    fn long_right_hand_values_never_overflow_58_or_80mm_paper() {
        for width in [32, 48] {
            let mut printer = Escpos::new(width);
            let amount = "Total PKR 12345678901234567890123456789012345678901234567890";
            printer.pair("Grand total including everything", amount);
            let printed = &printer.bytes[2..printer.bytes.len() - 1];
            assert_eq!(printed.len(), width);
            assert!(printed.ends_with(&amount.as_bytes()[amount.len() - width..]));
        }

        let mut printer = Escpos::new(32);
        printer.size(2);
        let start = printer.bytes.len();
        printer.pair("Grand total", "Rs 999999999999999999999");
        assert_eq!(printer.bytes.len() - start, 17);
    }

    #[test]
    fn thermal_slips_end_with_a_paper_cut_command() {
        let mut printer = Escpos::new(48);
        printer.cut();
        assert!(
            printer
                .bytes
                .ends_with(&[b'\n', b'\n', b'\n', 0x1d, b'V', 66, 0])
        );
    }

    #[test]
    fn long_unbroken_item_names_fit_the_receipt_width() {
        assert_eq!(
            wrap("ABCDEFGHIJKLMNOPQRSTUVWXYZ", 8),
            vec!["ABCDEFGH", "IJKLMNOP", "QRSTUVWX", "YZ"]
        );
        assert_eq!(
            wrap("Karahi ABCDEFGHIJKLMNOP", 8),
            vec!["Karahi", "ABCDEFGH", "IJKLMNOP"]
        );
    }

    #[test]
    fn wrapping_never_exceeds_requested_columns() {
        for width in [8, 16, 32, 48] {
            let lines = wrap(
                "Large-Customized-Item-Without-Spaces extra toppings and notes",
                width,
            );
            assert!(!lines.is_empty());
            assert!(lines.iter().all(|line| line.len() <= width));
        }
        assert!(wrap("no room", 0).is_empty());
        assert!(wrap("   ", 32).is_empty());
    }

    #[test]
    fn unsupported_unicode_is_replaced_predictably() {
        assert_eq!(ascii_string("Chicken × 2 — hot"), "Chicken x 2 - hot");
        assert_eq!(ascii_string("کچن"), "???");
    }
}
