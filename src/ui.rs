use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{
    content_id, content_label, ordered_content_for_display, App, ContentPublicationState,
    InputTarget, LoadState, PendingConfirmation, Screen,
};
use crate::microcms::ContentCollectionKind;

pub fn draw(frame: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_status(frame, app, areas[0]);
    draw_main(frame, app, areas[1]);
    draw_help(frame, app, areas[2]);
    if app.input_target.is_some() {
        draw_input_modal(frame, app);
    }
    if app.pending_confirmation.is_some() {
        draw_confirmation_modal(frame, app);
    }
    if app.help_open {
        draw_help_modal(frame, app);
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let status = match app.screen {
        Screen::EndpointPicker => picker_status(app),
        Screen::ContentBrowser => content_status(app),
    };
    frame.render_widget(
        Paragraph::new(Line::from(status)).style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        area,
    );
}

fn picker_status(app: &App) -> String {
    let state = match &app.state {
        LoadState::MissingConfig(_) => "missing configuration".to_string(),
        LoadState::LoadingApis => "loading...".to_string(),
        LoadState::ApisLoaded => format!("{} available", app.apis.len()),
        LoadState::Error(error) => format!("error: {error}"),
        _ => format!("{} available", app.apis.len()),
    };
    format!(" microcms-tui | APIs | {state}")
}

fn content_status(app: &App) -> String {
    let endpoint = app.endpoint.as_deref().unwrap_or("<not set>");
    let mut state = match &app.state {
        LoadState::LoadingContents => "loading...".to_string(),
        LoadState::ContentsLoaded if app.content_kind == ContentCollectionKind::Object => {
            format!("object | showing {}", app.items.len())
        }
        LoadState::ContentsLoaded => format!(
            "{} total | showing {}-{}",
            app.total_count.unwrap_or(0),
            if app.items.is_empty() {
                0
            } else {
                app.offset + 1
            },
            app.offset + app.items.len()
        ),
        LoadState::Error(error) => format!("error: {error}"),
        _ => "ready".to_string(),
    };
    if let Some(message) = &app.message {
        state.push_str(" | ");
        state.push_str(message);
    }
    let mut query_state = Vec::new();
    if let Some(query) = &app.search_query {
        query_state.push(format!("q:{}", truncate_inline(query, 18)));
    }
    if app.filters.is_some() {
        query_state.push("filters:*".to_string());
    }
    if let Some(orders) = &app.orders {
        query_state.push(format!("orders:{}", truncate_inline(orders, 18)));
    }
    if !query_state.is_empty() {
        state.push_str(" | ");
        state.push_str(&query_state.join(" "));
    }
    if !app.selected_content_ids.is_empty() {
        state.push_str(&format!(" | selected:{}", app.selected_content_ids.len()));
    }
    format!(" microcms-tui | endpoint: {endpoint} | {state}")
}

fn truncate_inline(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let visible: String = value.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{visible}...")
    }
}

fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    if let LoadState::MissingConfig(missing) = &app.state {
        draw_missing_config(frame, missing, area);
        return;
    }

    match app.screen {
        Screen::EndpointPicker => draw_endpoint_picker(frame, app, area),
        Screen::ContentBrowser => draw_content_browser(frame, app, area),
    }
}

fn draw_missing_config(frame: &mut Frame, missing: &[String], area: Rect) {
    let mut lines = vec![
        Line::from("microcms-tui needs credentials before it can discover APIs."),
        Line::from(""),
        Line::from("Missing:"),
    ];
    lines.extend(missing.iter().map(|item| Line::from(format!("- {item}"))));
    lines.push(Line::from(""));
    lines.push(Line::from(
        "Provide CLI flags, environment variables, or save a config file with --save-config.",
    ));
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("Setup"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_endpoint_picker(frame: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let items: Vec<ListItem> = app
        .apis
        .iter()
        .map(|api| {
            let label = api.name.as_deref().map_or_else(
                || api.endpoint.clone(),
                |name| format!("{} - {name}", api.endpoint),
            );
            ListItem::new(label)
        })
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Endpoints"))
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let selected = (!app.apis.is_empty()).then_some(app.api_selected);
    let mut list_state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, columns[0], &mut list_state);

    let details = app
        .apis
        .get(app.api_selected)
        .map(|api| {
            format!(
                "Endpoint: {}\nName: {}\n\n{}",
                api.endpoint,
                api.name.as_deref().unwrap_or("<not set>"),
                api.description.as_deref().unwrap_or("No description.")
            )
        })
        .unwrap_or_else(|| match &app.state {
            LoadState::LoadingApis => "Loading APIs...".to_string(),
            LoadState::Error(_) => "Press r to retry API discovery.".to_string(),
            _ => "No APIs found.".to_string(),
        });
    frame.render_widget(
        Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false }),
        columns[1],
    );
}

fn draw_content_browser(frame: &mut Frame, app: &App, area: Rect) {
    if app.preview_fullscreen {
        draw_preview(frame, app, area, true);
        return;
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    draw_content_list(frame, app, columns[0]);
    draw_preview(frame, app, columns[1], false);
}

fn draw_content_list(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let is_selected = content_id(value)
                .map(|id| app.selected_content_ids.contains(id))
                .unwrap_or(false);
            let mut spans = vec![selected_bar(is_selected)];
            spans.extend(status_marker(app.publication_state_for(value)));
            spans.push(Span::raw(format!(
                "{:>4}  {}",
                app.offset + index + 1,
                content_label(value, &app.content_field_order)
            )));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let title = if app.content_kind == ContentCollectionKind::Object {
        "Object content".to_string()
    } else {
        format!("Contents (page size {})", app.limit)
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    let selected = (!app.items.is_empty()).then_some(app.content_selected);
    let mut list_state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn selected_bar(selected: bool) -> Span<'static> {
    if selected {
        Span::styled("┃ ", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("  ")
    }
}

fn status_marker(state: ContentPublicationState) -> Vec<Span<'static>> {
    let green = Style::default().fg(Color::Green);
    let cyan = Style::default().fg(Color::Cyan);
    match state {
        ContentPublicationState::Published => vec![Span::styled("●  ", green)],
        ContentPublicationState::Draft => vec![Span::styled("●  ", cyan)],
        ContentPublicationState::PublishedAndDraft => {
            vec![Span::styled("●", green), Span::styled("● ", cyan)]
        }
        ContentPublicationState::Closed => {
            vec![Span::styled("●  ", Style::default().fg(Color::Red))]
        }
        ContentPublicationState::Unknown => {
            vec![Span::styled("●  ", Style::default().fg(Color::DarkGray))]
        }
    }
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect, fullscreen: bool) {
    let preview = app
        .items
        .get(app.content_selected)
        .map(|value| ordered_content_for_display(value, &app.content_field_order, true))
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| "No content selected.".to_string());
    frame.render_widget(
        Paragraph::new(preview)
            .block(Block::default().borders(Borders::ALL).title(if fullscreen {
                "JSON preview (fullscreen)"
            } else {
                "JSON preview"
            }))
            .scroll((if fullscreen { app.preview_scroll } else { 0 }, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let help = footer_text(app);
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::Black).bg(Color::White)),
        area,
    );
}

fn footer_text(app: &App) -> &'static str {
    if app.input_target.is_some() {
        " Enter apply | Esc cancel"
    } else if app.pending_confirmation.is_some() {
        " y confirm | n/Esc cancel | ? help"
    } else if app.preview_fullscreen {
        " Enter/Esc close | n/p content | ? help"
    } else {
        match app.screen {
            Screen::EndpointPicker => " Enter select | ? help",
            Screen::ContentBrowser if app.content_kind == ContentCollectionKind::Object => {
                " Enter preview | ? help"
            }
            Screen::ContentBrowser => " Enter preview | n/p page | ? help",
        }
    }
}

fn draw_help_modal(frame: &mut Frame, app: &App) {
    let area = centered_modal(frame.area(), 82, 26);
    let help = if app.screen == Screen::ContentBrowser
        && app.content_kind == ContentCollectionKind::Object
    {
        Text::from(vec![
            Line::from("Object API (GET-only)"),
            Line::from("  ? open/close help"),
            Line::from(""),
            Line::from("Navigation"),
            Line::from("  Enter preview fullscreen   b, Esc back"),
            Line::from("  r reload"),
            Line::from(""),
            Line::from("Preview fullscreen"),
            Line::from("  j/k, Up/Down scroll        g/G top/bottom"),
            Line::from("  Enter, Esc close"),
            Line::from(""),
            Line::from("Query"),
            Line::from("  / search q   f filters   o orders   x clear query"),
            Line::from(""),
            Line::from("  Create, edit, delete, bulk, and status operations are unavailable."),
        ])
    } else {
        Text::from(vec![
            Line::from("Global"),
            Line::from("  ? open/close help"),
            Line::from(""),
            Line::from("Navigation"),
            Line::from("  j/k, Up/Down move list      Enter preview/select endpoint"),
            Line::from("  b, Esc back/close   r reload   n/p, PgDn/PgUp next/previous page"),
            Line::from(""),
            Line::from("Preview fullscreen"),
            Line::from("  j/k, Up/Down scroll         g/G top/bottom"),
            Line::from("  n/p previous/next content   Enter, Esc close"),
            Line::from(""),
            Line::from("Query"),
            Line::from("  / search q                  f filters"),
            Line::from("  o orders                    x clear query"),
            Line::from(""),
            Line::from("Content write"),
            Line::from("  c/C POST default/draft      u/U PUT with ID default/draft"),
            Line::from("  e/E PATCH default/draft"),
            Line::from(""),
            Line::from("Bulk / Status"),
            Line::from("  Space mark current          d delete marked/current"),
            Line::from("  P publish (Management API)  D draft (Management API)"),
        ])
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(help)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title("Help"),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_input_modal(frame: &mut Frame, app: &App) {
    let Some(target) = app.input_target else {
        return;
    };
    let title = match target {
        InputTarget::Search => "Search",
        InputTarget::Filters => "Filters",
        InputTarget::Orders => "Orders",
        InputTarget::CreateWithId(_) => "Content ID",
    };
    let area = centered_modal(frame.area(), 70, 3);
    let input = Text::from(Line::from(format!("{} _", app.input_buffer)));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(input)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(title),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_confirmation_modal(frame: &mut Frame, app: &App) {
    let Some(confirmation) = app.pending_confirmation.as_ref() else {
        return;
    };
    let (title, prompt, warning) = confirmation_text(app, confirmation);
    let area = centered_modal_with_max_width(frame.area(), 60, 7, 72);
    let confirmation = Text::from(vec![
        Line::from(prompt),
        Line::from(""),
        Line::from(warning),
        Line::from(""),
        Line::from("y confirm | n/Esc cancel"),
    ]);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(confirmation)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(title),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn confirmation_text(
    app: &App,
    confirmation: &PendingConfirmation,
) -> (&'static str, String, &'static str) {
    match confirmation {
        PendingConfirmation::Delete { content_ids } => {
            let prompt = if content_ids.len() > 1 {
                format!("Delete {} selected contents?", content_ids.len())
            } else {
                content_ids
                    .first()
                    .and_then(|pending_id| {
                        app.items
                            .iter()
                            .find(|value| content_id(value) == Some(pending_id.as_str()))
                    })
                    .map(|value| content_label(value, &app.content_field_order))
                    .map(|label| format!("Delete: {}", truncate_inline(&label, 48)))
                    .unwrap_or_else(|| "Delete selected content?".to_string())
            };
            ("Confirm delete", prompt, "This cannot be undone.")
        }
        PendingConfirmation::Create { .. } => (
            "Confirm publish",
            "Create and publish content?".into(),
            "This operation may change publication state.",
        ),
        PendingConfirmation::PutCreate { content_id, .. } => (
            "Confirm publish",
            format!(
                "Create and publish content with ID {}?",
                truncate_inline(content_id, 36)
            ),
            "This operation may change publication state.",
        ),
        PendingConfirmation::Update { .. } => (
            "Confirm save",
            "Save changes to published content?".into(),
            "This operation may change publication state.",
        ),
        PendingConfirmation::PublicationStatus {
            content_ids,
            status: crate::microcms::PublicationStatus::Publish,
        } => (
            "Confirm publish",
            if content_ids.len() == 1 {
                "Publish selected content?".into()
            } else {
                format!("Publish {} selected contents?", content_ids.len())
            },
            "This operation may change publication state.",
        ),
        PendingConfirmation::PublicationStatus {
            content_ids,
            status: crate::microcms::PublicationStatus::Draft,
        } => (
            "Confirm draft",
            if content_ids.len() == 1 {
                "Set selected content to draft?".into()
            } else {
                format!("Set {} selected contents to draft?", content_ids.len())
            },
            "This operation may change publication state.",
        ),
    }
}

fn centered_modal(area: Rect, percent_width: u16, height: u16) -> Rect {
    centered_modal_with_max_width(area, percent_width, height, 100)
}

fn centered_modal_with_max_width(
    area: Rect,
    percent_width: u16,
    height: u16,
    max_width: u16,
) -> Rect {
    let width = area
        .width
        .saturating_mul(percent_width)
        .checked_div(100)
        .unwrap_or(area.width)
        .min(max_width)
        .max(1)
        .min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_and_draft_marker_uses_two_aligned_dots() {
        let marker = status_marker(ContentPublicationState::PublishedAndDraft);
        assert_eq!(marker.len(), 2);
        assert_eq!(marker[0].content.as_ref(), "●");
        assert_eq!(marker[1].content.as_ref(), "● ");
    }

    #[test]
    fn selected_bar_is_yellow_or_equal_width_blank() {
        let selected = selected_bar(true);
        assert_eq!(selected.content.as_ref(), "┃ ");
        assert_eq!(selected.style.fg, Some(Color::Yellow));

        let unselected = selected_bar(false);
        assert_eq!(unselected.content.as_ref(), "  ");
        assert_eq!(unselected.style.fg, None);
    }

    #[test]
    fn footers_are_short_and_offer_help_except_during_input() {
        let mut app = App::new(crate::config::Config::default());
        assert_eq!(footer_text(&app), " Enter select | ? help");

        app.screen = Screen::ContentBrowser;
        assert!(footer_text(&app).contains("? help"));
        assert!(footer_text(&app).len() < 80);

        app.content_kind = ContentCollectionKind::Object;
        assert_eq!(footer_text(&app), " Enter preview | ? help");

        app.input_target = Some(InputTarget::Search);
        assert!(!footer_text(&app).contains("? help"));
        assert_eq!(footer_text(&app), " Enter apply | Esc cancel");
    }

    #[test]
    fn confirmation_text_matches_publish_draft_save_and_delete_operations() {
        let app = App::new(crate::config::Config::default());
        assert_eq!(
            confirmation_text(
                &app,
                &PendingConfirmation::Create {
                    value: serde_json::json!({}),
                    status: crate::microcms::ContentWriteStatus::Default,
                }
            )
            .0,
            "Confirm publish"
        );
        assert_eq!(
            confirmation_text(
                &app,
                &PendingConfirmation::Update {
                    content_id: "id".into(),
                    value: serde_json::json!({}),
                    status: crate::microcms::ContentWriteStatus::Default,
                }
            )
            .1,
            "Save changes to published content?"
        );
        assert_eq!(
            confirmation_text(
                &app,
                &PendingConfirmation::PublicationStatus {
                    content_ids: vec!["one".into(), "two".into()],
                    status: crate::microcms::PublicationStatus::Draft,
                }
            )
            .1,
            "Set 2 selected contents to draft?"
        );
        assert_eq!(
            confirmation_text(
                &app,
                &PendingConfirmation::Delete {
                    content_ids: vec!["id".into()],
                }
            )
            .0,
            "Confirm delete"
        );
    }
}
