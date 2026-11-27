//! The view's text fields, each given Mod+Shift+Enter: the keymap's shortcut
//! must be spent before the field sees it, so that it runs without also
//! typing or submitting in the field (`keymap::guard`).

use super::*;
use crate::settings::guard_testing::Fixture;

fn entry(title: &str) -> MemoryEntry {
    MemoryEntry {
        id: title.into(),
        title: title.into(),
        content: "body".into(),
        tags: Vec::new(),
        workspace: String::new(),
        agent: String::new(),
        created_at: 1,
        updated_at: 1,
    }
}

fn loaded(entries: Vec<MemoryEntry>) -> State {
    State {
        haystacks: entries.iter().map(guards::haystack).collect(),
        entries: Some(entries),
        ..State::default()
    }
}

#[test]
fn the_search_field_leaves_the_shortcut_to_the_keymap() {
    let fixture = Fixture::new();
    let st = loaded(vec![entry("Build steps")]);
    assert_eq!(fixture.split_chord(view(&st, fixture.ctx())), ["Noop"]);
}

#[test]
fn the_composer_s_title_text_and_tags_leave_it_too() {
    let fixture = Fixture::new();
    let st = State {
        composing: true,
        ..loaded(vec![entry("Build steps")])
    };
    // The title, the text and the tags of the window; the search is under
    // its veil and out of reach.
    assert_eq!(
        fixture.split_chord(view(&st, fixture.ctx())),
        ["Noop", "Noop", "Noop"]
    );
}

#[test]
fn the_fields_of_an_entry_being_edited_leave_it_too() {
    let fixture = Fixture::new();
    let one = entry("Build steps");
    let mut st = loaded(vec![one.clone()]);
    st.open.insert(one.id.clone());
    st.editing = Some(Editing {
        id: one.id.clone(),
        draft: Draft::of(&one),
    });
    // The entry's title, text and tags, in the window over the page.
    assert_eq!(
        fixture.split_chord(view(&st, fixture.ctx())),
        ["Noop", "Noop", "Noop"]
    );
}
