use anyhow::Result;
use ratatui::layout::{Constraint, Layout};
use ratatui::{DefaultTerminal, Frame};

use crate::output::{BrowserWidget, NavWidget, TextOverlay};

use super::app_state::AppState;
use super::timing::RenderConfig;

// ---------------------------------------------------------------------------
// draw_frame
// ---------------------------------------------------------------------------

/// Compose and flush one TUI frame.
pub fn draw_frame(
    terminal: &mut DefaultTerminal,
    app: &mut AppState,
    cfg: &RenderConfig,
) -> Result<()> {
    terminal.draw(|f: &mut Frame| {
        let [nav_area, browser_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(f.area());

        let nav = NavWidget::new(&app.nav);
        f.render_widget(nav, nav_area);

        if let Some(frame) = app.frame.as_ref() {
            f.render_widget(BrowserWidget::new(frame, cfg.true_color), browser_area);

            if let Some(dl) = app.display_list.as_ref() {
                let pixels = Some((frame.pixels.as_slice(), frame.size.x, frame.size.y));
                f.render_widget(
                    TextOverlay::new(
                        &dl.items,
                        app.window.cell_pixels,
                        pixels,
                        cfg.true_color,
                        frame.scroll_offset.x,
                        frame.scroll_offset.y,
                    ),
                    browser_area,
                );
            }
        }

        if let Some(pos) = nav.cursor_position(nav_area) {
            f.set_cursor_position(pos);
        }
    })?;

    Ok(())
}
