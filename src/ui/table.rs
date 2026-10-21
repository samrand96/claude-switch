//! Minimal column-aligned table that measures styled text correctly.

use console::{Alignment, measure_text_width, pad_str};

use super::dim;

pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new<I: IntoIterator<Item = S>, S: Into<String>>(headers: I) -> Self {
        Self {
            headers: headers.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, cells: I) {
        self.rows.push(cells.into_iter().map(Into::into).collect());
    }

    pub fn print(&self) {
        let cols = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| measure_text_width(h)).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate().take(cols) {
                widths[i] = widths[i].max(measure_text_width(cell));
            }
        }
        let render = |cells: &[String], header: bool| {
            let mut line = String::from("  ");
            for (i, width) in widths.iter().enumerate() {
                let cell = cells.get(i).map(String::as_str).unwrap_or("");
                let padded = pad_str(cell, *width, Alignment::Left, None).to_string();
                if header {
                    line.push_str(&dim(padded));
                } else {
                    line.push_str(&padded);
                }
                if i + 1 < widths.len() {
                    line.push_str("   ");
                }
            }
            line.trim_end().to_string()
        };
        println!("{}", render(&self.headers, true));
        for row in &self.rows {
            println!("{}", render(row, false));
        }
    }
}
