//! Unified truncation system for tool outputs.
//!
//! Provides head and tail truncation by line count and byte size,
//! with structured metadata about what was truncated.

/// Maximum lines returned from a file read.
pub const MAX_LINES: usize = 2000;

/// Maximum bytes returned from any tool output (~50 KB).
pub const MAX_BYTES: usize = 51_200;

/// Maximum characters per line in grep output.
pub const GREP_MAX_LINE_LENGTH: usize = 500;

/// Result of a truncation operation.
#[derive(Debug, Clone)]
pub struct TruncationResult {
    pub content: String,
    pub original_lines: usize,
    pub output_lines: usize,
    pub original_bytes: usize,
    pub output_bytes: usize,
    pub truncated_by: Option<TruncatedBy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TruncatedBy {
    Lines,
    Bytes,
}

impl TruncationResult {
    pub fn is_truncated(&self) -> bool {
        self.truncated_by.is_some()
    }

    pub fn truncation_notice(&self) -> Option<String> {
        match &self.truncated_by {
            Some(TruncatedBy::Lines) => Some(format!(
                "[Truncated: showing {} of {} lines]",
                self.output_lines, self.original_lines
            )),
            Some(TruncatedBy::Bytes) => Some(format!(
                "[Truncated: showing {} of {} bytes]",
                self.output_bytes, self.original_bytes
            )),
            None => None,
        }
    }
}

/// Keep the first `max_lines` lines and first `max_bytes` bytes.
/// Suitable for file reads (beginning of file matters).
pub fn truncate_head(input: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let original_bytes = input.len();
    let original_lines = input.lines().count();

    // Fast path: nothing to truncate
    if original_lines <= max_lines && original_bytes <= max_bytes {
        return TruncationResult {
            content: input.to_string(),
            original_lines,
            output_lines: original_lines,
            original_bytes,
            output_bytes: original_bytes,
            truncated_by: None,
        };
    }

    let mut output = String::new();
    let mut line_count = 0;
    let mut byte_count = 0;
    let mut truncated_by = None;

    for line in input.lines() {
        if line_count >= max_lines {
            truncated_by = Some(TruncatedBy::Lines);
            break;
        }

        let line_with_newline = if line_count > 0 {
            format!("\n{}", line)
        } else {
            line.to_string()
        };
        let new_bytes = byte_count + line_with_newline.len();

        if new_bytes > max_bytes {
            truncated_by = Some(TruncatedBy::Bytes);
            break;
        }

        output.push_str(&line_with_newline);
        byte_count = new_bytes;
        line_count += 1;
    }

    TruncationResult {
        original_lines,
        output_lines: line_count,
        original_bytes,
        output_bytes: output.len(),
        truncated_by,
        content: output,
    }
}

/// Keep the last `max_lines` lines and last `max_bytes` bytes.
/// Suitable for bash output (end/errors matter most).
pub fn truncate_tail(input: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let original_bytes = input.len();
    let original_lines = input.lines().count();

    // Fast path
    if original_lines <= max_lines && original_bytes <= max_bytes {
        return TruncationResult {
            content: input.to_string(),
            original_lines,
            output_lines: original_lines,
            original_bytes,
            output_bytes: original_bytes,
            truncated_by: None,
        };
    }

    let lines: Vec<&str> = input.lines().collect();
    let start = if lines.len() > max_lines {
        lines.len() - max_lines
    } else {
        0
    };

    let selected: Vec<&str> = lines[start..].to_vec();
    let mut joined = selected.join("\n");

    let truncated_by = if start > 0 {
        Some(TruncatedBy::Lines)
    } else {
        None
    };

    // Apply byte limit from the end
    let final_truncated_by = if joined.len() > max_bytes {
        let excess = joined.len() - max_bytes;
        // Find character boundary
        let mut cut = excess;
        while cut < joined.len() && !joined.is_char_boundary(cut) {
            cut += 1;
        }
        joined = joined[cut..].to_string();
        Some(TruncatedBy::Bytes)
    } else {
        truncated_by
    };

    let output_lines = joined.lines().count();

    TruncationResult {
        original_lines,
        output_lines,
        original_bytes,
        output_bytes: joined.len(),
        truncated_by: final_truncated_by,
        content: joined,
    }
}

/// Truncate a single line to max length, appending "..." if truncated.
pub fn truncate_line(line: &str, max_chars: usize) -> String {
    if line.len() <= max_chars {
        line.to_string()
    } else {
        let mut end = max_chars.saturating_sub(3);
        while end < line.len() && !line.is_char_boundary(end) {
            end += 1;
        }
        format!("{}...", &line[..end])
    }
}

/// Add line numbers to content (1-indexed, matching `cat -n` format).
pub fn add_line_numbers(content: &str, start_line: usize) -> String {
    content
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{:>6}\t{}", start_line + i, line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_no_truncation() {
        let input = "line1\nline2\nline3";
        let result = truncate_head(input, 10, MAX_BYTES);
        assert!(!result.is_truncated());
        assert_eq!(result.content, input);
        assert_eq!(result.original_lines, 3);
        assert_eq!(result.output_lines, 3);
    }

    #[test]
    fn head_truncate_by_lines() {
        let input = "a\nb\nc\nd\ne";
        let result = truncate_head(input, 3, MAX_BYTES);
        assert!(result.is_truncated());
        assert_eq!(result.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(result.output_lines, 3);
        assert_eq!(result.content, "a\nb\nc");
    }

    #[test]
    fn head_truncate_by_bytes() {
        let input = "hello world this is a longer string\nsecond line here";
        let result = truncate_head(input, MAX_LINES, 20);
        assert!(result.is_truncated());
        assert_eq!(result.truncated_by, Some(TruncatedBy::Bytes));
    }

    #[test]
    fn tail_no_truncation() {
        let input = "line1\nline2\nline3";
        let result = truncate_tail(input, 10, MAX_BYTES);
        assert!(!result.is_truncated());
        assert_eq!(result.content, input);
    }

    #[test]
    fn tail_truncate_by_lines() {
        let input = "a\nb\nc\nd\ne";
        let result = truncate_tail(input, 3, MAX_BYTES);
        assert!(result.is_truncated());
        assert_eq!(result.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(result.content, "c\nd\ne");
    }

    #[test]
    fn truncate_line_short() {
        assert_eq!(truncate_line("hello", 10), "hello");
    }

    #[test]
    fn truncate_line_long() {
        let long = "a".repeat(600);
        let result = truncate_line(&long, 500);
        assert!(result.len() <= 503); // 500 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn line_numbers() {
        let input = "first\nsecond\nthird";
        let result = add_line_numbers(input, 1);
        assert!(result.contains("     1\tfirst"));
        assert!(result.contains("     2\tsecond"));
        assert!(result.contains("     3\tthird"));
    }

    #[test]
    fn line_numbers_with_offset() {
        let input = "line10\nline11";
        let result = add_line_numbers(input, 10);
        assert!(result.contains("    10\tline10"));
        assert!(result.contains("    11\tline11"));
    }

    #[test]
    fn truncation_notice() {
        let result = truncate_head("a\nb\nc\nd\ne", 3, MAX_BYTES);
        let notice = result.truncation_notice().unwrap();
        assert!(notice.contains("3 of 5 lines"));
    }

    #[test]
    fn empty_input() {
        let result = truncate_head("", MAX_LINES, MAX_BYTES);
        assert!(!result.is_truncated());
        assert_eq!(result.content, "");
    }
}
