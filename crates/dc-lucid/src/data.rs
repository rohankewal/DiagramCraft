//! The CSV data sources a package carries in its `data` directory.
//!
//! The format publishes the directory and says the files in it are CSV, but
//! it publishes no schema binding a row to a shape. So this module stops at
//! reading a source into headers and rows; deciding which row belongs to
//! which shape is the editor's business, and what it writes there is
//! `customData`, which the format does publish.
//!
//! The parser is written here rather than taken as a dependency because the
//! surface needed is this small: separator, quoting, and line endings.

/// One CSV file from the package's `data` directory.
#[derive(Clone, Debug, PartialEq)]
pub struct DataSource {
    /// The entry name relative to `data/`, such as `records.csv`.
    pub name: String,
    /// The first row, which the format's use of CSV treats as the header.
    pub headers: Vec<String>,
    /// Every row after the header, each padded to the header's width so a
    /// short row cannot panic a caller indexing by column.
    pub rows: Vec<Vec<String>>,
}

impl DataSource {
    /// Reads a source, or `None` when it holds no header row.
    ///
    /// Invalid UTF-8 is read lossily rather than refused: a spreadsheet
    /// export with one bad byte is still worth linking against.
    pub fn parse(name: impl Into<String>, bytes: &[u8]) -> Option<DataSource> {
        let text = String::from_utf8_lossy(bytes);
        let mut records = parse_records(&text);
        if records.is_empty() {
            return None;
        }
        let headers = records.remove(0);
        if headers.is_empty() {
            return None;
        }

        // A short row reads as empty in the missing columns, and a long one
        // keeps what it has; neither is worth discarding the row over.
        for row in &mut records {
            if row.len() < headers.len() {
                row.resize(headers.len(), String::new());
            }
        }

        Some(DataSource { name: name.into(), headers, rows: records })
    }

    /// The index of a column, matched without regard to case or surrounding
    /// space, since a header written by hand rarely agrees exactly.
    pub fn column(&self, header: &str) -> Option<usize> {
        let wanted = header.trim().to_lowercase();
        self.headers
            .iter()
            .position(|h| h.trim().to_lowercase() == wanted)
    }

    /// The first row whose `column` holds `value`, compared the same
    /// forgiving way as a header.
    pub fn row_matching(&self, column: usize, value: &str) -> Option<&[String]> {
        let wanted = value.trim().to_lowercase();
        self.rows
            .iter()
            .find(|row| {
                row.get(column)
                    .is_some_and(|cell| cell.trim().to_lowercase() == wanted)
            })
            .map(|row| row.as_slice())
    }
}

/// Splits CSV text into records.
///
/// Handles the three things a real export does that a naive split does not:
/// quoted fields holding separators or newlines, a doubled quote standing for
/// one inside a quoted field, and either line ending.
fn parse_records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        quoted = false;
                    }
                }
                _ => field.push(c),
            }
            continue;
        }

        match c {
            '"' => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' | '\n' => {
                // Takes CRLF as one ending rather than as a blank record.
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut field));
                // A blank line carries no record.
                if !(record.len() == 1 && record[0].is_empty()) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            _ => field.push(c),
        }
    }

    // Whatever the last line left, when the file did not end in a newline.
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }

    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_headers_and_rows() {
        let src = DataSource::parse("records.csv", b"id,name\n1,ops\n2,sales\n").unwrap();
        assert_eq!(src.name, "records.csv");
        assert_eq!(src.headers, vec!["id", "name"]);
        assert_eq!(src.rows.len(), 2);
        assert_eq!(src.rows[1], vec!["2", "sales"]);
    }

    #[test]
    fn a_file_with_no_header_is_not_a_source() {
        assert!(DataSource::parse("empty.csv", b"").is_none());
    }

    #[test]
    fn a_header_alone_is_a_source_with_no_rows() {
        let src = DataSource::parse("records.csv", b"id,name").unwrap();
        assert_eq!(src.headers, vec!["id", "name"]);
        assert!(src.rows.is_empty());
    }

    #[test]
    fn quoted_fields_keep_their_separators() {
        let src = DataSource::parse("r.csv", b"id,note\n1,\"ops, and more\"\n").unwrap();
        assert_eq!(src.rows[0], vec!["1", "ops, and more"]);
    }

    #[test]
    fn a_doubled_quote_stands_for_one() {
        let src = DataSource::parse("r.csv", b"id,note\n1,\"say \"\"hi\"\"\"\n").unwrap();
        assert_eq!(src.rows[0], vec!["1", "say \"hi\""]);
    }

    #[test]
    fn a_quoted_field_may_hold_a_newline() {
        let src = DataSource::parse("r.csv", b"id,note\n1,\"two\nlines\"\n").unwrap();
        assert_eq!(src.rows.len(), 1, "the embedded newline is not a record break");
        assert_eq!(src.rows[0][1], "two\nlines");
    }

    #[test]
    fn carriage_returns_do_not_survive_into_fields() {
        let src = DataSource::parse("r.csv", b"id,name\r\n1,ops\r\n").unwrap();
        assert_eq!(src.headers, vec!["id", "name"]);
        assert_eq!(src.rows[0], vec!["1", "ops"]);
    }

    #[test]
    fn a_short_row_is_padded_to_the_header_width() {
        let src = DataSource::parse("r.csv", b"id,name,owner\n1,ops\n").unwrap();
        assert_eq!(src.rows[0], vec!["1", "ops", ""]);
    }

    #[test]
    fn a_file_not_ending_in_a_newline_keeps_its_last_row() {
        let src = DataSource::parse("r.csv", b"id,name\n1,ops").unwrap();
        assert_eq!(src.rows.len(), 1);
        assert_eq!(src.rows[0], vec!["1", "ops"]);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let src = DataSource::parse("r.csv", b"id,name\n\n1,ops\n\n").unwrap();
        assert_eq!(src.rows.len(), 1);
    }

    #[test]
    fn columns_and_rows_are_found_without_regard_to_case() {
        let src = DataSource::parse("r.csv", b"Id,Name\nA1,ops\n").unwrap();
        assert_eq!(src.column("id"), Some(0));
        assert_eq!(src.column(" NAME "), Some(1));
        assert_eq!(src.column("missing"), None);
        assert_eq!(src.row_matching(0, "a1").unwrap()[1], "ops");
        assert!(src.row_matching(0, "nope").is_none());
    }

    #[test]
    fn invalid_utf8_is_read_rather_than_refused() {
        let src = DataSource::parse("r.csv", b"id,name\n1,op\xffs\n").unwrap();
        assert_eq!(src.rows.len(), 1, "the row survives the bad byte");
    }
}
