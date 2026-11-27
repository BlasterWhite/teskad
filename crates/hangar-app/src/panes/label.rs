//! A line of words cut at its end to the room it has: the pane header's
//! branch, node and task.

use iced::Font;

/// `words` as they fit in `width` on one line in this size and face: whole,
/// or cut at the end with "…" in the last place.
pub fn fit_end(words: &str, width: f32, size: f32, font: Font) -> String {
    let fits = |s: &str| hangar_ui::text_width(s, size, font) <= width;
    if fits(words) {
        return words.to_owned();
    }
    let ends: Vec<usize> = words.char_indices().map(|(at, _)| at).collect();
    let cut = |kept: usize| format!("{}…", words[..ends[kept]].trim_end());
    // The most characters that still fit with the "…" after them.
    let (mut low, mut high) = (0, ends.len().saturating_sub(1));
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(&cut(middle)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    if ends.is_empty() {
        String::new()
    } else {
        cut(low)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_that_fit_stay_whole_and_the_others_end_in_an_ellipsis() {
        let font = hangar_theme::fonts::mono();
        assert_eq!(fit_end("main", 112.0, 11.0, font), "main");
        let cut = fit_end("teskad/refonte-5-8-ecran-01", 112.0, 11.0, font);
        assert!(cut.starts_with("teskad/") && cut.ends_with('…'), "{cut}");
        assert!(hangar_ui::text_width(&cut, 11.0, font) <= 112.0);
        // Cut between characters, never inside one.
        assert!(fit_end("éééééééééééééééééééééééé", 40.0, 11.0, font).ends_with('…'));
        assert_eq!(fit_end("main", 0.0, 11.0, font), "…");
    }
}
