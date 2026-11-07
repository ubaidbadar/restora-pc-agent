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
        let room = width.saturating_sub(right.len() + 1);
        self.bytes.extend(&left[..left.len().min(room)]);
        let gap = width.saturating_sub(left.len().min(room) + right.len());
        self.bytes.extend(std::iter::repeat_n(b' ', gap));
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
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in ascii_string(text).split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
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
            assert_eq!(&printer.bytes[2..], format!("{}\\n", "-".repeat(width)).as_bytes());
        }
    }

    #[test]
    fn double_size_uses_half_the_paper_columns() {
        let mut printer = Escpos::new(32);
        printer.size(2);
        printer.rule('-');
        assert!(printer.bytes.ends_with(format!("{}\\n", "-".repeat(16)).as_bytes()));
    }

    #[test]
    fn monetary_columns_are_kept_right_aligned() {
        let mut printer = Escpos::new(32);
        printer.pair("Total", "Rs 2,900");
        assert!(printer.bytes.ends_with(b"Total                   Rs 2,900\\n"));
    }

    #[test]
    fn thermal_slips_end_with_a_paper_cut_command() {
        let mut printer = Escpos::new(48);
        printer.cut();
        assert!(printer.bytes.ends_with(&[b'\\n', b'\\n', b'\\n', 0x1d, b'V', 66, 0]));
    }

    #[test]
    fn unsupported_unicode_is_replaced_predictably() {
        assert_eq!(ascii_string("Chicken × 2 — hot"), "Chicken x 2 - hot");
        assert_eq!(ascii_string("کچن"), "???");
    }
}
