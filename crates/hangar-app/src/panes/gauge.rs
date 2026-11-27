//! How full the agent's context window is (`PaneGauge.tsx`): a 28 px bar and
//! a percentage, with the numbers behind them in the tooltip. Amber from
//! 70 %, red from 90 %: the agent compacts on its own near the limit, and
//! these say how near that is. The bar itself is hangar-ui's.

use hangar_i18n::{Key, args};
use iced::Element;

use crate::ctx::Ctx;
use crate::message::Message;
use crate::panes_logic::format_tokens;
use crate::state::live::ContextGauge;
use crate::state::types::AgentId;

/// The gauge of a pane header. `bar` is off in a narrow header, which keeps
/// the figure alone.
pub fn view<'a>(
    ctx: Ctx<'a>,
    gauge: &ContextGauge,
    agent: AgentId,
    bar: bool,
) -> Element<'a, Message> {
    let pct = gauge.pct.round().clamp(0.0, 100.0) as u8;
    let model = gauge
        .model
        .clone()
        .unwrap_or_else(|| agent.as_str().to_owned());
    let words = ctx.i18n.t_with(
        Key::PaneContext,
        &args!(
            "used" => format_tokens(gauge.used_tokens),
            "window" => format_tokens(gauge.window),
            "pct" => u64::from(pct),
            "model" => model,
        ),
    );
    hangar_ui::meter::context(ctx.theme, pct)
        .bar(bar)
        .tip(words.into_owned())
        .into()
}
