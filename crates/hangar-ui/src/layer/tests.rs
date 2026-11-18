use super::*;

const WINDOW: Size = Size::new(800.0, 600.0);

fn anchor(x: f32, y: f32) -> Rectangle {
    Rectangle::new(Point::new(x, y), Size::new(80.0, 26.0))
}

mod position {
    use super::*;

    #[test]
    fn a_dialog_is_centred() {
        let at = position(
            Placement::Center,
            Rectangle::with_size(Size::ZERO),
            Size::new(400.0, 200.0),
            WINDOW,
            4.0,
            24.0,
        );
        assert_eq!(at, Point::new(200.0, 200.0));
    }

    #[test]
    fn a_dialog_larger_than_the_window_starts_at_its_corner() {
        let at = position(
            Placement::Center,
            Rectangle::with_size(Size::ZERO),
            Size::new(900.0, 700.0),
            WINDOW,
            4.0,
            24.0,
        );
        assert_eq!(at, Point::ORIGIN);
    }

    #[test]
    fn the_palette_sits_a_fraction_down_and_centred_across() {
        let at = position(
            Placement::Top(0.14),
            Rectangle::with_size(Size::ZERO),
            Size::new(640.0, 300.0),
            WINDOW,
            4.0,
            16.0,
        );
        assert_eq!(at.x, 80.0);
        assert!((at.y - 84.0).abs() < 1e-3);
    }

    #[test]
    fn a_tall_palette_is_pulled_up_to_stay_in_the_window() {
        let at = position(
            Placement::Top(0.14),
            Rectangle::with_size(Size::ZERO),
            Size::new(640.0, 560.0),
            WINDOW,
            4.0,
            16.0,
        );
        // 600 - 560 - 16 = 24: lower than 84 would leave it off the bottom.
        assert_eq!(at.y, 24.0);
    }

    #[test]
    fn a_menu_opens_under_its_button_at_the_buttons_left_edge() {
        let at = position(
            Placement::Below { align_end: false },
            anchor(100.0, 50.0),
            Size::new(200.0, 150.0),
            WINDOW,
            4.0,
            8.0,
        );
        assert_eq!(at, Point::new(100.0, 80.0));
    }

    #[test]
    fn a_menu_can_end_at_its_buttons_right_edge() {
        let at = position(
            Placement::Below { align_end: true },
            anchor(700.0, 10.0),
            Size::new(200.0, 150.0),
            WINDOW,
            4.0,
            8.0,
        );
        // The button ends at 780; the menu ends there too.
        assert_eq!(at.x, 580.0);
    }

    #[test]
    fn a_menu_flips_above_its_button_when_there_is_no_room_under_it() {
        let at = position(
            Placement::Below { align_end: false },
            anchor(100.0, 550.0),
            Size::new(200.0, 150.0),
            WINDOW,
            4.0,
            8.0,
        );
        assert_eq!(at.y, 550.0 - 4.0 - 150.0);
    }

    #[test]
    fn a_menu_stays_inside_the_window_sideways() {
        let at = position(
            Placement::Below { align_end: false },
            anchor(760.0, 10.0),
            Size::new(200.0, 100.0),
            WINDOW,
            4.0,
            8.0,
        );
        assert_eq!(at.x, 800.0 - 200.0 - 8.0);
        let left = position(
            Placement::Below { align_end: true },
            anchor(0.0, 10.0),
            Size::new(200.0, 100.0),
            WINDOW,
            4.0,
            8.0,
        );
        assert_eq!(left.x, 8.0);
    }

    #[test]
    fn a_context_menu_opens_at_the_pointer() {
        let at = position(
            Placement::At(Vector::new(30.0, 12.0)),
            anchor(100.0, 200.0),
            Size::new(220.0, 180.0),
            WINDOW,
            4.0,
            8.0,
        );
        assert_eq!(at, Point::new(130.0, 212.0));
    }

    #[test]
    fn a_context_menu_near_an_edge_is_pulled_back_inside() {
        let at = position(
            Placement::At(Vector::new(70.0, 20.0)),
            anchor(700.0, 560.0),
            Size::new(220.0, 180.0),
            WINDOW,
            4.0,
            8.0,
        );
        assert_eq!(at, Point::new(800.0 - 220.0 - 8.0, 600.0 - 180.0 - 8.0));
    }

    #[test]
    fn a_dock_sits_in_the_bottom_right_corner_a_margin_from_the_edges() {
        let at = position(
            Placement::BottomEnd,
            Rectangle::with_size(Size::ZERO),
            Size::new(460.0, 270.0),
            WINDOW,
            4.0,
            16.0,
        );
        assert_eq!(at, Point::new(800.0 - 460.0 - 16.0, 600.0 - 270.0 - 16.0));
    }

    #[test]
    fn a_dock_larger_than_the_window_is_pinned_to_the_margin() {
        let at = position(
            Placement::BottomEnd,
            Rectangle::with_size(Size::ZERO),
            Size::new(900.0, 700.0),
            WINDOW,
            4.0,
            16.0,
        );
        assert_eq!(at, Point::new(16.0, 16.0));
    }
}

mod dock_interaction {
    use iced::widget::{button, container};
    use iced::{Element, Length};

    use super::*;
    use crate::testing::{Drive, simulate};

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Page,
        Dock,
        Close,
    }

    fn page<'a>() -> Element<'a, Msg> {
        button("page")
            .width(Length::Fill)
            .height(Length::Fill)
            .on_press(Msg::Page)
            .into()
    }

    fn panel<'a>() -> Element<'a, Msg> {
        container(button("dock").width(200).height(100).on_press(Msg::Dock)).into()
    }

    fn messages(ui: iced_test::Simulator<'_, Msg>) -> Vec<Msg> {
        ui.into_messages().collect()
    }

    #[test]
    fn the_window_under_a_dock_stays_reachable() {
        let layer = dock(page(), Some(panel())).on_dismiss(Msg::Close);
        let mut ui = simulate(Element::from(layer));
        ui.click_at(40.0, 40.0);
        assert_eq!(messages(ui), [Msg::Page]);
    }

    #[test]
    fn the_dock_itself_takes_its_clicks_in_the_corner() {
        let layer = dock(page(), Some(panel())).on_dismiss(Msg::Close);
        let mut ui = simulate(Element::from(layer));
        // 800 x 600 window, 200 x 100 panel, 16 px from the edges.
        ui.click_at(800.0 - 16.0 - 100.0, 600.0 - 16.0 - 50.0);
        assert_eq!(messages(ui), [Msg::Dock]);
    }

    #[test]
    fn escape_still_dismisses_a_dock() {
        let layer = dock(page(), Some(panel())).on_dismiss(Msg::Close);
        let mut ui = simulate(Element::from(layer));
        ui.key(iced::keyboard::key::Named::Escape);
        assert_eq!(messages(ui), [Msg::Close]);
    }

    #[test]
    fn a_modal_blocks_the_same_click() {
        let layer = modal(page(), Some(panel())).on_dismiss(Msg::Close);
        let mut ui = simulate(Element::from(layer));
        ui.click_at(40.0, 40.0);
        assert!(!messages(ui).contains(&Msg::Page));
    }
}

mod stacking {
    use iced::widget::{container, scrollable};
    use iced::{Color, Element, Length};

    use super::*;
    use crate::testing::simulate;

    fn block<'a>(color: Color, width: f32, height: f32) -> Element<'a, ()> {
        container("")
            .width(width)
            .height(height)
            .style(move |_| container::background(color))
            .into()
    }

    /// A dialog opened over a dialog that holds a scrollable is drawn over
    /// the scrollable's content, which iced draws in a render layer of its own.
    #[test]
    fn a_dialog_over_a_dialog_with_a_scrollable_hides_the_scrolled_content() {
        const RED: Color = Color::from_rgb(1.0, 0.0, 0.0);
        const BLUE: Color = Color::from_rgb(0.0, 0.0, 1.0);

        let lower = scrollable(block(RED, 400.0, 800.0))
            .width(Length::Fixed(400.0))
            .height(Length::Fixed(400.0));
        let under = modal(Element::from(container("")), Some(Element::from(lower)));
        let over = modal(Element::from(under), Some(block(BLUE, 100.0, 100.0)));

        let mut ui = simulate(Element::from(over));
        let snapshot = ui.snapshot(&iced::Theme::Dark).expect("it draws");
        let dir = crate::testdir::scratch("layer");
        snapshot
            .matches_image(dir.join("stacked"))
            .expect("written");
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .next()
            .expect("the png")
            .path();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(file).unwrap()))
                .read_info()
                .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        // The snapshot is drawn at twice the size; the dialog is at the centre.
        let pixel = |x: u32, y: u32| {
            let at = (y as usize * info.width as usize + x as usize) * 4;
            &bytes[at..at + 3]
        };
        let (cx, cy) = (info.width / 2, info.height / 2);
        // Inside the upper dialog: blue, never the red of the one under it.
        assert_eq!(pixel(cx, cy), [0, 0, 255], "the upper dialog is opaque");
        assert_eq!(pixel(cx + 60, cy), [0, 0, 255]);
        // Beside it, the lower dialog's content is under the veil.
        let beside = pixel(cx + 2 * 150, cy);
        assert!(beside[0] > beside[2], "the lower dialog shows around it");
    }

    /// The shell's case: two dialogs are siblings in a `stack`, not nested.
    /// Their overlays are grouped at one level; the later one still wins.
    #[test]
    fn a_sibling_dialog_over_a_sibling_with_a_scrollable_hides_the_scrolled_content() {
        const RED: Color = Color::from_rgb(1.0, 0.0, 0.0);
        const BLUE: Color = Color::from_rgb(0.0, 0.0, 1.0);

        let lower = scrollable(block(RED, 400.0, 800.0))
            .width(Length::Fixed(400.0))
            .height(Length::Fixed(400.0));
        let under = modal(Element::from(container("")), Some(Element::from(lower)));
        let over = modal(
            Element::from(container("")),
            Some(block(BLUE, 100.0, 100.0)),
        );
        let both = iced::widget::stack![Element::from(under), Element::from(over)];

        let mut ui = simulate(Element::from(both));
        let snapshot = ui.snapshot(&iced::Theme::Dark).expect("it draws");
        let dir = crate::testdir::scratch("layer-sib");
        snapshot
            .matches_image(dir.join("stacked"))
            .expect("written");
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .next()
            .expect("the png")
            .path();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(file).unwrap()))
                .read_info()
                .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let pixel = |x: u32, y: u32| {
            let at = (y as usize * info.width as usize + x as usize) * 4;
            &bytes[at..at + 3]
        };
        let (cx, cy) = (info.width / 2, info.height / 2);
        assert_eq!(pixel(cx, cy), [0, 0, 255], "the later dialog is on top");
        assert_eq!(pixel(cx + 60, cy), [0, 0, 255]);
    }
}

mod shadows {
    use iced::widget::container;
    use iced::{Color, Element, Shadow, Vector};

    use super::*;
    use crate::testing::simulate;

    /// A layer without a veil is only as big as its card, yet its shadow
    /// reaches outside the card: the render layer must not clip it.
    #[test]
    fn a_dock_keeps_the_shadow_it_casts_outside_its_card() {
        let card: Element<'_, ()> = container("")
            .width(200.0)
            .height(100.0)
            .style(|_| {
                container::background(Color::WHITE).shadow(Shadow {
                    color: Color::BLACK,
                    offset: Vector::ZERO,
                    blur_radius: 16.0,
                })
            })
            .into();
        let page: Element<'_, ()> = container("")
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .style(|_| container::background(Color::WHITE))
            .into();

        let mut ui = simulate(Element::from(dock(page, Some(card))));
        let snapshot = ui.snapshot(&iced::Theme::Light).expect("it draws");
        let dir = crate::testdir::scratch("layer-shadow");
        snapshot.matches_image(dir.join("shadow")).expect("written");
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .next()
            .expect("the png")
            .path();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(file).unwrap()))
                .read_info()
                .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let pixel = |x: u32, y: u32| {
            let at = (y as usize * info.width as usize + x as usize) * 4;
            bytes[at]
        };
        // The snapshot is at twice the size. The dock is a margin (16) from
        // the bottom-right corner; probe 3 px outside its left edge, halfway
        // down, and a point well away from it.
        let scale = 2.0;
        let (w, h) = (info.width as f32 / scale, info.height as f32 / scale);
        let left = w - 16.0 - 200.0;
        let middle = h - 16.0 - 50.0;
        let near = pixel(((left - 3.0) * scale) as u32, (middle * scale) as u32);
        let far = pixel(8, 8);
        assert_eq!(far, 255, "the page is white away from the card");
        assert!(near < 250, "the shadow shows beside the card, got {near}");
    }
}

mod focus {
    use iced::widget::text_input;
    use iced::{Element, Event, mouse};
    use iced_test::core::clipboard;
    use iced_test::core::widget::{self, Operation, operation::Focusable};
    use iced_test::runtime::UserInterface;
    use iced_test::runtime::user_interface::Cache;

    use super::*;
    use crate::testing::WINDOW;

    /// Writes down the ids of the fields that hold the keyboard.
    #[derive(Default)]
    struct Focused(Vec<String>);

    impl Operation for Focused {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn focusable(
            &mut self,
            id: Option<&widget::Id>,
            _bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            if state.is_focused() {
                self.0.push(format!("{:?}", id));
            }
        }
    }

    fn view<'a>(modal_open: bool) -> Element<'a, ()> {
        let page = text_input("page", "").id("page");
        let content = modal_open.then(|| Element::from(text_input("dialog", "").id("dialog")));
        modal(page, content).into()
    }

    fn focused(
        ui: &mut UserInterface<'_, (), iced::Theme, iced::Renderer>,
        r: &iced::Renderer,
    ) -> Vec<String> {
        let mut op = Focused::default();
        ui.operate(r, &mut op);
        op.0
    }

    #[test]
    fn a_field_focused_under_a_modal_loses_the_focus_and_the_dialog_takes_it() {
        let mut renderer = iced_test::futures::futures::executor::block_on(
            <iced::Renderer as iced_test::core::renderer::Headless>::new(
                iced::Font::DEFAULT,
                iced::Pixels(16.0),
                std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
            ),
        )
        .expect("a headless renderer");

        // The page alone, with its field focused.
        let mut ui = UserInterface::build(view(false), WINDOW, Cache::default(), &mut renderer);
        let mut focus = widget::operation::focusable::focus(widget::Id::new("page"));
        ui.operate(&renderer, &mut focus);
        assert_eq!(focused(&mut ui, &renderer).len(), 1);

        // The dialog opens over it; the next event reaches the overlay first.
        let cache = ui.into_cache();
        let mut ui = UserInterface::build(view(true), WINDOW, cache, &mut renderer);
        let mut messages = Vec::new();
        ui.update(
            &[Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(400.0, 300.0),
            })],
            mouse::Cursor::Available(Point::new(400.0, 300.0)),
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        assert_eq!(
            focused(&mut ui, &renderer),
            ["Some(Id(Custom(\"dialog\")))"]
        );
    }
}

/// Two dialogs as the shell stacks them: siblings in a `stack`, the later one
/// in front. iced hands each event to both, the one under first; the one under
/// is drawn through [`inert`], and only the one in front answers.
mod covered {
    use iced::widget::{Space, button, column, container, stack, text_input};
    use iced::{Element, Event, window};

    use super::*;
    use crate::testing::{Drive, simulate};

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Under,
        UnderTyped(String),
        CloseUnder,
        Accept,
        FrontTyped(String),
        CloseFront,
    }

    /// The dialog under: a button 300 x 120 in the middle of the 800 x 600
    /// window (x 250 to 550, y 240 to 360), and nothing else.
    fn under<'a>() -> Element<'a, Msg> {
        let card = button("under").width(300).height(120).on_press(Msg::Under);
        modal(Element::from(container("")), Some(card.into()))
            .on_dismiss(Msg::CloseUnder)
            .into()
    }

    /// The dialog in front: taller than the one under it (300 x 400, y 100 to
    /// 500), its button at its foot (y 440 to 480), below the card under it,
    /// as an activation's Accept is below a workspace's Kits dialog.
    fn front<'a>() -> Element<'a, Msg> {
        let card = container(column![
            Space::new().height(340.0),
            button("accept").width(200).height(40).on_press(Msg::Accept),
        ])
        .width(300)
        .height(400);
        modal(Element::from(container("")), Some(card.into()))
            .on_dismiss(Msg::CloseFront)
            .into()
    }

    fn both<'a>(covered: bool) -> Element<'a, Msg> {
        stack![inert(under()).when(covered), front()].into()
    }

    fn messages(ui: iced_test::Simulator<'_, Msg>) -> Vec<Msg> {
        ui.into_messages().collect()
    }

    #[test]
    fn bare_the_dialog_under_takes_a_click_below_its_card_and_asks_to_close() {
        // What the shell had before: the veil under, pressed outside its own
        // card, takes the press from the button in front.
        let mut ui = simulate(both(false));
        ui.click_at(350.0, 460.0);
        assert_eq!(messages(ui), [Msg::CloseUnder]);
    }

    #[test]
    fn covered_the_button_in_front_gets_its_click() {
        let mut ui = simulate(both(true));
        ui.click_at(350.0, 460.0);
        assert_eq!(messages(ui), [Msg::Accept]);
    }

    #[test]
    fn covered_a_button_under_the_card_in_front_is_not_reached() {
        let mut ui = simulate(both(true));
        // On the card in front, over the button of the one under it.
        ui.click_at(400.0, 300.0);
        assert_eq!(messages(ui), Vec::<Msg>::new());
    }

    #[test]
    fn covered_escape_closes_the_one_in_front() {
        let mut ui = simulate(both(true));
        ui.key(iced::keyboard::key::Named::Escape);
        assert_eq!(messages(ui), [Msg::CloseFront]);
    }

    #[test]
    fn covered_the_veil_in_front_closes_the_one_in_front() {
        let mut ui = simulate(both(true));
        ui.click_at(40.0, 40.0);
        assert_eq!(messages(ui), [Msg::CloseFront]);
    }

    #[test]
    fn uncovered_it_is_left_as_it_is() {
        let ui_alone = |covered: bool| {
            let mut ui = simulate(Element::from(inert(under()).when(covered)));
            ui.click_at(400.0, 300.0);
            messages(ui)
        };
        assert_eq!(ui_alone(false), [Msg::Under]);
        assert_eq!(ui_alone(true), Vec::<Msg>::new());
    }

    /// Two fields, one in each dialog, both holding the keyboard: each dialog
    /// gives it to its first field when it opens.
    fn fields<'a>() -> Element<'a, Msg> {
        let under = modal(
            Element::from(container("")),
            Some(
                text_input("under", "")
                    .on_input(Msg::UnderTyped)
                    .width(300)
                    .into(),
            ),
        )
        .on_dismiss(Msg::CloseUnder);
        let front = modal(
            Element::from(container("")),
            Some(
                text_input("front", "")
                    .on_input(Msg::FrontTyped)
                    .width(300)
                    .into(),
            ),
        )
        .on_dismiss(Msg::CloseFront);
        stack![inert(under), front].into()
    }

    #[test]
    fn covered_its_field_types_nothing_and_the_field_in_front_types() {
        let mut ui = simulate(fields());
        // A frame drawn: the one under is told to redraw, which is when a
        // dialog that never had an event takes the keyboard for its field.
        ui.simulate([Event::Window(window::Event::RedrawRequested(
            iced_test::core::time::Instant::now(),
        ))]);
        let _ = ui.typewrite("a");
        assert_eq!(messages(ui), [Msg::FrontTyped("a".into())]);
    }
}
