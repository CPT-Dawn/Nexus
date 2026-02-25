pub mod theme;
pub mod components;
pub mod pages;

use ratatui::Frame;
use crate::app::App;

/// Top-level draw function — dispatches to the active page
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    pages::render(f, app, area);
}
