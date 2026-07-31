use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{
    content_id, content_label, ordered_content_for_display, App, ContentPublicationState,
    InputTarget, LoadState, PendingConfirmation, QuerySelector, ReservationField, Screen,
    VersionView,
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
    if app.query_selector.is_some() {
        draw_query_selector_modal(frame, app);
    }
    if app.reservation_input.is_some() {
        draw_reservation_modal(frame, app);
    }
    if app.version_comparison.is_some() {
        draw_version_comparison(frame, app);
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
    if app.fields.is_some() {
        query_state.push("fields:*".to_string());
    }
    if let Some(depth) = app.depth {
        query_state.push(format!("depth:{depth}"));
    }
    if app.ids.is_some() {
        query_state.push("ids:*".to_string());
    }
    if app.draft_key.is_some() {
        query_state.push("draftKey:*".to_string());
    }
    if let Some(format) = &app.rich_editor_format {
        query_state.push(format!("richEditorFormat:{format}"));
    }
    if !query_state.is_empty() {
        state.push_str(" | ");
        state.push_str(&query_state.join(" "));
    }
    if !app.selected_content_ids.is_empty() {
        state.push_str(&format!(" | selected:{}", app.selected_content_ids.len()));
    }
    if let Some(reservation) = app
        .items
        .get(app.content_selected)
        .and_then(|value| app.reservation_for(value))
    {
        state.push_str(" | scheduled:");
        state.push_str(&reservation_summary(reservation));
    }
    format!(" microcms-tui | endpoint: {endpoint} | {state}")
}

fn reservation_summary(reservation: &crate::microcms::ReservationTime) -> String {
    let publish = reservation
        .publish_time
        .as_deref()
        .map(|value| truncate_inline(value, 20));
    let stop = reservation
        .stop_time
        .as_deref()
        .map(|value| truncate_inline(value, 20));
    match (publish, stop) {
        (Some(publish), Some(stop)) => format!("publish {publish}, stop {stop}"),
        (Some(publish), None) => format!("publish {publish}"),
        (None, Some(stop)) => format!("stop {stop}"),
        (None, None) => "none".into(),
    }
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
            let kind = api
                .kind
                .or_else(|| app.content_kind_cache.get(&api.endpoint).copied());
            let label = api.name.as_deref().map_or_else(
                || api.endpoint.clone(),
                |name| format!("{} - {name}", api.endpoint),
            );
            ListItem::new(format!("{} {label}", endpoint_kind_icon(kind)))
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
            let kind = api
                .kind
                .or_else(|| app.content_kind_cache.get(&api.endpoint).copied());
            format!(
                "Endpoint: {}\nName: {}\nType: {}\n\n{}",
                api.endpoint,
                api.name.as_deref().unwrap_or("<not set>"),
                match kind {
                    Some(ContentCollectionKind::List) => "list",
                    Some(ContentCollectionKind::Object) => "object",
                    None => "unknown",
                },
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

fn endpoint_kind_icon(kind: Option<ContentCollectionKind>) -> &'static str {
    match kind {
        Some(ContentCollectionKind::Object) => "\u{e60b}",
        Some(ContentCollectionKind::List) => "\u{f0ca}",
        None => " ",
    }
}

fn draw_content_browser(frame: &mut Frame, app: &App, area: Rect) {
    if app.content_kind == ContentCollectionKind::Object {
        draw_preview(frame, app, area, false);
        return;
    }
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
            spans.extend(content_status_marker(
                app.publication_state_for(value),
                app.reservation_for(value).is_some(),
            ));
            spans.push(Span::raw(format!(
                "{:>4}  {}",
                app.offset + index + 1,
                content_label(value, &app.content_field_order)
            )));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let title = format!("Contents (page size {})", app.limit);
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

fn scheduled_status_marker() -> Span<'static> {
    Span::styled("● ", Style::default().fg(Color::Magenta))
}

fn content_status_marker(
    publication_state: ContentPublicationState,
    scheduled: bool,
) -> Vec<Span<'static>> {
    if !scheduled {
        return status_marker(publication_state);
    }
    let mut markers = match publication_state {
        ContentPublicationState::Published => {
            vec![Span::styled("●", Style::default().fg(Color::Green))]
        }
        ContentPublicationState::Draft => {
            vec![Span::styled("●", Style::default().fg(Color::Cyan))]
        }
        ContentPublicationState::PublishedAndDraft => vec![
            Span::styled("●", Style::default().fg(Color::Green)),
            Span::styled("●", Style::default().fg(Color::Cyan)),
        ],
        ContentPublicationState::Closed => {
            vec![Span::styled("●", Style::default().fg(Color::Red))]
        }
        ContentPublicationState::Unknown => {
            vec![Span::styled("●", Style::default().fg(Color::DarkGray))]
        }
    };
    markers.push(scheduled_status_marker());
    markers
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect, fullscreen: bool) {
    let object = app.content_kind == ContentCollectionKind::Object;
    let preview = app
        .items
        .get(app.content_selected)
        .map(|value| ordered_content_for_display(value, &app.content_field_order, true))
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| "No content selected.".to_string());
    frame.render_widget(
        Paragraph::new(preview)
            .block(Block::default().borders(Borders::ALL).title(if object {
                "Object JSON preview"
            } else if fullscreen {
                "JSON preview (fullscreen)"
            } else {
                "JSON preview"
            }))
            .scroll((app.preview_scroll, 0))
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
    } else if matches!(app.query_selector, Some(QuerySelector::Fields { .. })) {
        " j/k move | Space toggle | Enter apply | Esc cancel"
    } else if app.query_selector.is_some() {
        " j/k choose | Enter apply | Esc cancel"
    } else if app.reservation_input.is_some() {
        " Tab field | Enter review | F8 clear | Esc cancel"
    } else if app.pending_confirmation.is_some() {
        " y confirm | n/Esc cancel | ? help"
    } else if app.version_comparison.is_some() {
        " 1 published | 2 draft | j/k scroll | Enter/Esc close"
    } else if app.preview_fullscreen {
        " Enter/Esc close | n/p content | ? help"
    } else {
        match app.screen {
            Screen::EndpointPicker => " Enter select | ? help",
            Screen::ContentBrowser if app.content_kind == ContentCollectionKind::Object => {
                " j/k scroll | g/G top/bottom | r reload | b/Esc back | ? help"
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
            Line::from("JSON preview"),
            Line::from("  j/k, Up/Down scroll        g/G top/bottom"),
            Line::from("  r reload                   b, Esc endpoints"),
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
            Line::from("  o orders   l fields selector   z depth selector"),
            Line::from("  i IDs   K draftKey   m format selector   x clear"),
            Line::from(""),
            Line::from("Content write"),
            Line::from("  c/C POST default/draft      u/U PUT with ID default/draft"),
            Line::from("  e/E PATCH default/draft"),
            Line::from(""),
            Line::from("Bulk / Status"),
            Line::from("  Space mark current          d delete marked/current"),
            Line::from("  P publish (Management API)  D draft (Management API)"),
            Line::from("  s publication reservation   v published/draft comparison"),
        ])
    };
    clear_modal_background(frame, area);
    frame.render_widget(
        Paragraph::new(help)
            .style(modal_style())
            .block(
                Block::default()
                    .style(modal_style())
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
        InputTarget::Ids => "IDs",
        InputTarget::DraftKey => "Draft key",
        InputTarget::CreateWithId(_) => "Content ID",
    };
    let area = centered_modal(frame.area(), 70, 3);
    let inner_width = area.width.saturating_sub(2) as usize;
    let (input, cursor_column) = visible_input(&app.input_buffer, app.input_cursor, inner_width);
    clear_modal_background(frame, area);
    frame.render_widget(
        Paragraph::new(input)
            .style(modal_style())
            .block(
                Block::default()
                    .style(modal_style())
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(title),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
    frame.set_cursor_position((area.x + 1 + cursor_column, area.y + 1));
}

fn draw_query_selector_modal(frame: &mut Frame, app: &App) {
    let Some(selector) = app.query_selector.as_ref() else {
        return;
    };
    let (title, cursor, entries): (&str, usize, Vec<Line<'static>>) = match selector {
        QuerySelector::Fields { cursor, selected } => (
            "Fields",
            *cursor,
            app.content_field_order
                .iter()
                .map(|field| {
                    Line::from(format!(
                        "[{}] {field}",
                        if selected.contains(field) { "x" } else { " " }
                    ))
                })
                .collect(),
        ),
        QuerySelector::Depth { cursor } => (
            "Depth",
            *cursor,
            ["unset", "0", "1", "2", "3"]
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    Line::from(format!(
                        "({}) {value}",
                        if index == *cursor { "*" } else { " " }
                    ))
                })
                .collect(),
        ),
        QuerySelector::RichEditorFormat { cursor } => (
            "Rich editor format",
            *cursor,
            ["unset", "html", "object"]
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    Line::from(format!(
                        "({}) {value}",
                        if index == *cursor { "*" } else { " " }
                    ))
                })
                .collect(),
        ),
    };
    let height = (entries.len() as u16 + 2).clamp(5, 20);
    let area = centered_modal_with_max_width(frame.area(), 64, height, 72);
    let visible_rows = area.height.saturating_sub(2) as usize;
    let first = cursor.saturating_add(1).saturating_sub(visible_rows.max(1));
    let visible = entries
        .into_iter()
        .skip(first)
        .take(visible_rows)
        .enumerate()
        .map(|(index, line)| {
            if first + index == cursor {
                line.style(Style::default().fg(Color::Yellow))
            } else {
                line
            }
        })
        .collect::<Vec<_>>();
    clear_modal_background(frame, area);
    frame.render_widget(
        Paragraph::new(visible).style(modal_style()).block(
            Block::default()
                .style(modal_style())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(title),
        ),
        area,
    );
}

fn draw_reservation_modal(frame: &mut Frame, app: &App) {
    let Some(input) = app.reservation_input.as_ref() else {
        return;
    };
    let (status, rule) = reservation_status_hint(input.publication_state);
    let area = centered_modal_with_max_width(frame.area(), 76, 10, 82);
    let publish_label = "Publish time: ";
    let stop_label = "Stop time:    ";
    let field_width = area.width.saturating_sub(2 + publish_label.len() as u16) as usize;
    let (publish_time, publish_cursor) =
        visible_input(&input.publish_time, input.publish_cursor, field_width);
    let (stop_time, stop_cursor) = visible_input(&input.stop_time, input.stop_cursor, field_width);
    let field_line = |label: &'static str, value: String, active: bool| {
        Line::from(vec![
            Span::styled(
                label,
                if active {
                    Style::default().fg(Color::Yellow)
                } else {
                    modal_style()
                },
            ),
            Span::raw(value),
        ])
    };
    let text = Text::from(vec![
        Line::from(format!("Current status: {status}")),
        field_line(
            publish_label,
            publish_time,
            input.active_field == ReservationField::PublishTime,
        ),
        field_line(
            stop_label,
            stop_time,
            input.active_field == ReservationField::StopTime,
        ),
        Line::from(""),
        Line::from("Format: YYYY-MM-DD HH:MM (local time) or ISO 8601"),
        Line::from(rule),
        Line::from("Tab field | Enter review | F8 clear | Esc cancel"),
    ]);
    clear_modal_background(frame, area);
    frame.render_widget(
        Paragraph::new(text)
            .style(modal_style())
            .block(
                Block::default()
                    .style(modal_style())
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title("Publication reservation"),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
    if !app.help_open {
        let (line, cursor) = match input.active_field {
            ReservationField::PublishTime => (area.y + 2, publish_cursor),
            ReservationField::StopTime => (area.y + 3, stop_cursor),
        };
        frame.set_cursor_position((area.x + 1 + publish_label.len() as u16 + cursor, line));
    }
}

fn visible_input(value: &str, cursor: usize, max_width: usize) -> (String, u16) {
    let chars: Vec<char> = value.chars().collect();
    let cursor = cursor.min(chars.len());
    let width = |character: char| UnicodeWidthChar::width(character).unwrap_or(0);
    let mut start = cursor;
    let mut cursor_width = 0;
    while start > 0 {
        let character_width = width(chars[start - 1]);
        if cursor_width + character_width >= max_width.max(1) {
            break;
        }
        cursor_width += character_width;
        start -= 1;
    }
    let mut visible_width = 0;
    let visible: String = chars[start..]
        .iter()
        .copied()
        .take_while(|character| {
            let character_width = width(*character);
            if visible_width + character_width > max_width {
                false
            } else {
                visible_width += character_width;
                true
            }
        })
        .collect();
    (visible, cursor_width as u16)
}

fn reservation_status_hint(state: ContentPublicationState) -> (&'static str, &'static str) {
    match state {
        ContentPublicationState::Published => (
            "published",
            "Published: set stop only, or stop first and the next publish later.",
        ),
        ContentPublicationState::Draft => (
            "draft",
            "Draft: set publish only, or publish first and stop later.",
        ),
        ContentPublicationState::Closed => (
            "closed",
            "Closed: set publish only, or publish first and stop later.",
        ),
        ContentPublicationState::PublishedAndDraft => (
            "published + draft",
            "Start/end availability depends on the current published and draft versions.",
        ),
        ContentPublicationState::Unknown => (
            "unknown",
            "Reservation availability will be validated by the Management API.",
        ),
    }
}

fn draw_version_comparison(frame: &mut Frame, app: &App) {
    let Some(comparison) = app.version_comparison.as_ref() else {
        return;
    };
    let area = centered_percent_rect(frame.area(), 92, 88);
    let (title, body) = match comparison.view {
        VersionView::Published => (
            "Published version",
            serde_json::to_string_pretty(&ordered_content_for_display(
                &comparison.published,
                &app.content_field_order,
                true,
            ))
            .unwrap_or_else(|error| format!("Failed to render JSON: {error}")),
        ),
        VersionView::Draft => (
            "Draft version",
            serde_json::to_string_pretty(&ordered_content_for_display(
                &comparison.draft,
                &app.content_field_order,
                true,
            ))
            .unwrap_or_else(|error| format!("Failed to render JSON: {error}")),
        ),
    };
    clear_modal_background(frame, area);
    frame.render_widget(
        Paragraph::new(body)
            .style(modal_style())
            .scroll((comparison.scroll, 0))
            .block(
                Block::default()
                    .style(modal_style())
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(format!("{title} | 1 published | 2 draft")),
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
    clear_modal_background(frame, area);
    frame.render_widget(
        Paragraph::new(confirmation)
            .style(modal_style())
            .block(
                Block::default()
                    .style(modal_style())
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
        PendingConfirmation::Reservation {
            publish_time,
            stop_time,
            ..
        } => (
            "Confirm reservation",
            if publish_time.is_none() && stop_time.is_none() {
                "Remove the current publication reservation?".into()
            } else {
                format!(
                    "Set reservation: publish {}, stop {}?",
                    publish_time.as_deref().unwrap_or("-"),
                    stop_time.as_deref().unwrap_or("-")
                )
            },
            "Existing reservation settings will be overwritten.",
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

fn modal_style() -> Style {
    Style::default().fg(Color::White).bg(Color::Black)
}

fn clear_modal_background(frame: &mut Frame, area: Rect) {
    let frame_area = frame.area();
    clear_wide_char_crossing_left_border(frame.buffer_mut(), frame_area, area);
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(modal_style()), area);
}

fn clear_wide_char_crossing_left_border(buffer: &mut Buffer, frame_area: Rect, modal_area: Rect) {
    if modal_area.x <= frame_area.x {
        return;
    }
    let scan_start = modal_area.x.saturating_sub(4).max(frame_area.x);
    let bottom = modal_area
        .y
        .saturating_add(modal_area.height)
        .min(frame_area.y.saturating_add(frame_area.height));
    for y in modal_area.y..bottom {
        for x in scan_start..modal_area.x {
            let width = UnicodeWidthStr::width(buffer[(x, y)].symbol()) as u16;
            if width > 1 && x.saturating_add(width) > modal_area.x {
                for occupied_x in x..modal_area.x {
                    buffer[(occupied_x, y)].reset();
                }
            }
        }
    }
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

fn centered_percent_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let width = area
        .width
        .saturating_mul(width_percent)
        .saturating_div(100)
        .max(1);
    let height = area
        .height
        .saturating_mul(height_percent)
        .saturating_div(100)
        .max(1);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

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
    fn scheduled_status_appends_an_adjacent_magenta_dot() {
        let scheduled = content_status_marker(ContentPublicationState::PublishedAndDraft, true);
        assert_eq!(scheduled.len(), 3);
        assert_eq!(scheduled[0].content.as_ref(), "●");
        assert_eq!(scheduled[0].style.fg, Some(Color::Green));
        assert_eq!(scheduled[1].content.as_ref(), "●");
        assert_eq!(scheduled[1].style.fg, Some(Color::Cyan));
        assert_eq!(scheduled[2].content.as_ref(), "● ");
        assert_eq!(scheduled[2].style.fg, Some(Color::Magenta));
    }

    #[test]
    fn reservation_summary_supports_start_end_and_both() {
        use crate::microcms::ReservationTime;

        assert_eq!(
            reservation_summary(&ReservationTime {
                publish_time: Some("2026-08-01T00:00:00Z".into()),
                stop_time: None,
            }),
            "publish 2026-08-01T00:00:00Z"
        );
        assert!(reservation_summary(&ReservationTime {
            publish_time: Some("start".into()),
            stop_time: Some("stop".into()),
        })
        .contains("publish start, stop stop"));
    }

    #[test]
    fn footers_are_short_and_offer_help_except_during_input() {
        let mut app = App::new(crate::config::Config::default());
        assert_eq!(footer_text(&app), " Enter select | ? help");

        app.screen = Screen::ContentBrowser;
        assert!(footer_text(&app).contains("? help"));
        assert!(footer_text(&app).len() < 80);

        app.content_kind = ContentCollectionKind::Object;
        assert_eq!(
            footer_text(&app),
            " j/k scroll | g/G top/bottom | r reload | b/Esc back | ? help"
        );

        app.input_target = Some(InputTarget::Search);
        assert!(!footer_text(&app).contains("? help"));
        assert_eq!(footer_text(&app), " Enter apply | Esc cancel");

        app.input_target = None;
        app.query_selector = Some(QuerySelector::Fields {
            cursor: 0,
            selected: Default::default(),
        });
        assert_eq!(
            footer_text(&app),
            " j/k move | Space toggle | Enter apply | Esc cancel"
        );
    }

    #[test]
    fn endpoint_kind_icons_use_requested_nerd_font_codepoints() {
        assert_eq!(
            endpoint_kind_icon(Some(ContentCollectionKind::Object)),
            "\u{e60b}"
        );
        assert_eq!(
            endpoint_kind_icon(Some(ContentCollectionKind::List)),
            "\u{f0ca}"
        );
        assert_eq!(endpoint_kind_icon(None), " ");
    }

    #[test]
    fn modal_background_is_opaque_and_right_border_remains_visible() {
        let mut app = App::new(crate::config::Config::default());
        app.screen = Screen::ContentBrowser;
        app.input_target = Some(InputTarget::Search);
        app.input_buffer = "query".into();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let area = centered_modal(Rect::new(0, 0, 80, 24), 70, 3);
        let buffer = terminal.backend().buffer();
        let right_border = buffer.cell((area.right() - 1, area.y + 1)).unwrap();
        assert_eq!(right_border.symbol(), "│");
        assert_eq!(right_border.bg, Color::Black);
        let interior = buffer.cell((area.x + 1, area.y + 1)).unwrap();
        assert_eq!(interior.bg, Color::Black);
    }

    #[test]
    fn modal_boundary_repairs_hiragana_katakana_and_kanji_without_moving_the_boundary() {
        let frame = Rect::new(0, 0, 20, 4);
        let modal = Rect::new(10, 1, 8, 2);

        for glyph in ["あ", "カ", "漢"] {
            let mut buffer = Buffer::empty(frame);
            buffer.set_string(modal.x - 1, modal.y, glyph, Style::default());
            clear_wide_char_crossing_left_border(&mut buffer, frame, modal);
            assert_eq!(buffer[(modal.x - 1, modal.y)].symbol(), " ", "{glyph}");
        }

        let mut buffer = Buffer::empty(frame);
        buffer.set_string(modal.x - 1, modal.y, "a", Style::default());
        clear_wide_char_crossing_left_border(&mut buffer, frame, modal);
        assert_eq!(buffer[(modal.x - 1, modal.y)].symbol(), "a");
    }

    #[test]
    fn modal_keeps_non_overlapping_text_and_draws_its_left_border() {
        let mut app = App::new(crate::config::Config::default());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.content_field_order = vec!["body".into()];
        app.items = (0..12)
            .map(|index| serde_json::json!({"id": format!("id-{index}"), "body": "あいうえおかきくけこ"}))
            .collect();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        app.input_target = Some(InputTarget::Search);
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let modal = centered_modal(Rect::new(0, 0, 80, 24), 70, 3);
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer.cell((modal.x - 1, modal.y + 1)).unwrap().symbol(),
            "0"
        );
        assert_eq!(buffer.cell((modal.x, modal.y + 1)).unwrap().symbol(), "│");
    }

    #[test]
    fn input_view_uses_real_cursor_without_fake_underscore_or_padding() {
        let (visible, cursor) = visible_input("b6h70np_q", 9, 20);
        assert_eq!(visible, "b6h70np_q");
        assert_eq!(cursor, 9);
        assert!(!visible.ends_with(" _"));

        let (visible, cursor) = visible_input("日本語abc", 2, 20);
        assert_eq!(visible, "日本語abc");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn fields_modal_renders_schema_candidates_as_checkboxes() {
        let mut app = App::new(crate::config::Config::default());
        app.screen = Screen::ContentBrowser;
        app.content_field_order = vec!["title".into(), "body".into()];
        app.query_selector = Some(QuerySelector::Fields {
            cursor: 0,
            selected: ["title".to_string()].into_iter().collect(),
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let area = centered_modal_with_max_width(Rect::new(0, 0, 80, 24), 64, 5, 72);
        let buffer = terminal.backend().buffer();
        let first: String = (area.x + 1..area.x + 10)
            .map(|x| buffer.cell((x, area.y + 1)).unwrap().symbol())
            .collect();
        let second: String = (area.x + 1..area.x + 9)
            .map(|x| buffer.cell((x, area.y + 2)).unwrap().symbol())
            .collect();
        assert_eq!(first, "[x] title");
        assert_eq!(second, "[ ] body");
    }

    #[test]
    fn object_api_renders_only_a_full_width_json_preview() {
        let mut app = App::new(crate::config::Config::default());
        app.screen = Screen::ContentBrowser;
        app.content_kind = ContentCollectionKind::Object;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![serde_json::json!({"body": "Object body"})];
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let rendered = (0..24)
            .map(|y| {
                (0..80)
                    .map(|x| buffer.cell((x, y)).unwrap().symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Object JSON preview"));
        assert!(!rendered.contains("Contents (page size"));
        assert!(!rendered.contains("Object content"));
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
        assert_eq!(
            confirmation_text(
                &app,
                &PendingConfirmation::Reservation {
                    content_id: "id".into(),
                    publish_time: None,
                    stop_time: None,
                }
            )
            .1,
            "Remove the current publication reservation?"
        );
    }
}
