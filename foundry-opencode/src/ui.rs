use color_eyre::eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Gauge, List, Paragraph, Wrap},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn screen(
    frame: &mut Frame,
    title: &str,
    text: &str,
    buttons: &[(&str, bool)],
    scroll: u16,
    help: &str,
) -> Vec<Rect> {
    let areas = sections(frame.area());
    frame.render_widget(
        Paragraph::new(format!(" Foundry OpenCode  /  {title}"))
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::BOTTOM)),
        areas[0],
    );
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(Block::default().borders(Borders::ALL).title(title)),
        areas[1],
    );
    let hits: Vec<_> = if buttons.len() == 1 {
        vec![
            Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(areas[2])
                [1],
        ]
    } else {
        Layout::horizontal(
            buttons
                .iter()
                .map(|_| Constraint::Ratio(1, buttons.len() as u32))
                .collect::<Vec<_>>(),
        )
        .split(areas[2])
        .iter()
        .copied()
        .collect()
    };
    for ((label, selected), area) in buttons.iter().zip(&hits) {
        let style = if *selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        frame.render_widget(
            Paragraph::new(format!(" {label} "))
                .style(style)
                .block(Block::default().borders(Borders::ALL)),
            *area,
        );
    }
    frame.render_widget(
        Paragraph::new(format!(" {help}")).style(Style::default().fg(Color::Gray)),
        areas[3],
    );
    hits
}

fn sections(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(area)
}

fn click(event: &Event, buttons: &[Rect]) -> Option<usize> {
    if let Event::Mouse(mouse) = event
        && mouse.kind == MouseEventKind::Down(MouseButton::Left)
    {
        return buttons
            .iter()
            .position(|area| area.contains(Position::new(mouse.column, mouse.row)));
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Back,
    Next,
    Cancel,
    Open,
}

fn back(first: bool) -> Nav {
    if first { Nav::Cancel } else { Nav::Back }
}

fn text_action(focus: usize, first: bool, open: bool) -> Nav {
    match focus {
        1 => back(first),
        2 if open => Nav::Open,
        _ => Nav::Next,
    }
}

fn directory_suggestions(input: &str, home: &Path) -> Vec<String> {
    if input == "~" {
        return vec!["~/".into()];
    }
    let (prefix, fragment) = input.rsplit_once('/').unwrap_or(("", input));
    let parent = if prefix == "~" {
        home.to_path_buf()
    } else if let Some(rest) = prefix.strip_prefix("~/") {
        home.join(rest)
    } else if input.starts_with('/') && prefix.is_empty() {
        PathBuf::from("/")
    } else if prefix.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(prefix)
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut suggestions: Vec<_> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            if !entry.path().is_dir()
                || name.chars().any(char::is_control)
                || (!fragment.starts_with('.') && name.starts_with('.'))
                || !name.to_lowercase().starts_with(&fragment.to_lowercase())
            {
                return None;
            }
            Some(if input.contains('/') {
                if prefix.is_empty() {
                    format!("/{name}")
                } else {
                    format!("{prefix}/{name}")
                }
            } else {
                name
            })
        })
        .collect();
    suggestions.sort_by_key(|path| path.to_lowercase());
    suggestions.truncate(5);
    suggestions
}

#[derive(Clone, Copy)]
pub enum Field<'a> {
    Plain,
    Token,
    Directory(&'a Path),
}

pub fn step_text(
    terminal: &mut DefaultTerminal,
    title: &str,
    prompt: &str,
    value: &mut String,
    first: bool,
    kind: Field<'_>,
) -> Result<Nav> {
    let secret = matches!(kind, Field::Token);
    let open = matches!(kind, Field::Token);
    let directory_home = if let Field::Directory(home) = kind {
        Some(home)
    } else {
        None
    };
    let mut focus = 0usize; // input, Back, optional Open, Continue
    let mut scroll = 0u16;
    let mut selected = 0usize;
    loop {
        let suggestions = directory_home
            .map(|home| directory_suggestions(value, home))
            .unwrap_or_default();
        selected = selected.min(suggestions.len().saturating_sub(1));
        let shown = if secret {
            format!(
                "{} ({} characters)",
                "•".repeat(value.chars().count().min(24)),
                value.chars().count()
            )
        } else {
            value.clone()
        };
        let actions = if open {
            vec![
                (if first { "Cancel" } else { "Back" }, focus == 1),
                ("Open token page", focus == 2),
                ("Continue", focus == 3),
            ]
        } else {
            vec![
                (if first { "Cancel" } else { "Back" }, focus == 1),
                ("Continue", focus == 2),
            ]
        };
        let mut buttons = Vec::new();
        let mut field = Rect::default();
        let mut choices = Rect::default();
        terminal.draw(|frame| {
            buttons = screen(
                frame,
                title,
                if directory_home.is_some() { "" } else { prompt },
                &actions,
                if directory_home.is_some() { 0 } else { scroll },
                if directory_home.is_some() {
                    "Type path  Up/Down: suggest  Tab: fill  Enter: continue  Esc: back"
                } else {
                    "Type: edit   Up/Down: scroll help   Tab: focus   Enter: activate   Esc: back"
                },
            );
            let body = sections(frame.area())[1];
            field = Rect::new(
                body.x + 1,
                body.bottom().saturating_sub(4),
                body.width.saturating_sub(2),
                3,
            );
            if directory_home.is_some() {
                let height = field.y.saturating_sub(body.y + 2).min(7);
                choices = Rect::new(field.x, field.y.saturating_sub(height), field.width, height);
                let explanation = Rect::new(
                    field.x,
                    body.y + 1,
                    field.width,
                    choices.y.saturating_sub(body.y + 1),
                );
                frame.render_widget(
                    Paragraph::new(prompt)
                        .wrap(Wrap { trim: false })
                        .scroll((scroll, 0)),
                    explanation,
                );
                let items = suggestions
                    .iter()
                    .enumerate()
                    .map(|(index, path)| {
                        format!("{} {path}", if index == selected { ">" } else { " " })
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(
                    List::new(items).block(
                        Block::default()
                            .title("Directory suggestions")
                            .borders(Borders::ALL),
                    ),
                    choices,
                );
            }
            let max_chars = field.width.saturating_sub(5) as usize;
            let visible: String = shown
                .chars()
                .rev()
                .take(max_chars)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            frame.render_widget(
                Paragraph::new(format!("{visible}{}", if focus == 0 { "▏" } else { "" }))
                    .block(Block::default().title("Input").borders(Borders::ALL)),
                field,
            );
        })?;
        let event = event::read()?;
        if let Some(index) = click(&event, &buttons) {
            return Ok(match index {
                0 => back(first),
                1 if open => Nav::Open,
                _ => Nav::Next,
            });
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && directory_home.is_some()
            && choices.contains(Position::new(mouse.column, mouse.row))
        {
            let index = mouse.row.saturating_sub(choices.y + 1) as usize;
            if let Some(path) = suggestions.get(index) {
                *value = path.clone();
                selected = 0;
                focus = 0;
            }
            continue;
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && field.contains(Position::new(mouse.column, mouse.row))
        {
            focus = 0;
        }
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc => return Ok(back(first)),
                KeyCode::Enter => {
                    return Ok(text_action(focus, first, open));
                }
                KeyCode::Tab
                    if directory_home.is_some() && focus == 0 && !suggestions.is_empty() =>
                {
                    *value = suggestions[selected].clone();
                    selected = 0;
                }
                KeyCode::Tab => focus = (focus + 1) % (actions.len() + 1),
                KeyCode::BackTab => focus = (focus + actions.len()) % (actions.len() + 1),
                KeyCode::Down if directory_home.is_some() => {
                    selected = (selected + 1).min(suggestions.len().saturating_sub(1))
                }
                KeyCode::Up if directory_home.is_some() => selected = selected.saturating_sub(1),
                KeyCode::PageDown if directory_home.is_some() => scroll = scroll.saturating_add(3),
                KeyCode::PageUp if directory_home.is_some() => scroll = scroll.saturating_sub(3),
                KeyCode::Down => scroll = scroll.saturating_add(1),
                KeyCode::Up => scroll = scroll.saturating_sub(1),
                KeyCode::Backspace if focus == 0 => {
                    value.pop();
                    selected = 0;
                }
                KeyCode::Char(c) if focus == 0 && !c.is_control() => {
                    value.push(c);
                    selected = 0;
                }
                _ => {}
            }
        }
    }
}

pub fn step_checkbox(
    terminal: &mut DefaultTerminal,
    title: &str,
    prompt: &str,
    label: &str,
    value: &mut bool,
    first: bool,
) -> Result<Nav> {
    loop {
        let mut buttons = Vec::new();
        let mut box_area = Rect::default();
        terminal.draw(|frame| {
            buttons = screen(
                frame,
                title,
                prompt,
                &[
                    (if first { "Cancel" } else { "Back" }, false),
                    ("Continue", true),
                ],
                0,
                "Space: toggle checkbox   Enter: continue   Esc: back",
            );
            let body = sections(frame.area())[1];
            box_area = Rect::new(
                body.x + 1,
                body.y + body.height / 2,
                body.width.saturating_sub(2).min((label.len() + 4) as u16),
                1,
            );
            frame.render_widget(
                Paragraph::new(format!("{} {label}", if *value { "[x]" } else { "[ ]" })).style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                box_area,
            );
        })?;
        let event = event::read()?;
        if let Some(index) = click(&event, &buttons) {
            return Ok(if index == 0 { back(first) } else { Nav::Next });
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && box_area.contains(Position::new(mouse.column, mouse.row))
        {
            *value = !*value;
        }
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc => return Ok(back(first)),
                KeyCode::Char(' ') => *value = !*value,
                KeyCode::Enter => return Ok(Nav::Next),
                _ => {}
            }
        }
    }
}

fn matches(names: &[String], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.to_lowercase().contains(&query))
        .map(|(index, _)| index)
        .collect()
}

fn stack_key(
    key: KeyCode,
    names: &[String],
    checked: &mut Vec<String>,
    query: &mut String,
    selected: &mut usize,
    scroll: &mut usize,
) {
    let filtered = matches(names, query);
    match key {
        KeyCode::Down | KeyCode::Tab => {
            *selected = (*selected + 1).min(filtered.len().saturating_sub(1))
        }
        KeyCode::Up | KeyCode::BackTab => *selected = selected.saturating_sub(1),
        KeyCode::Backspace => {
            query.pop();
            *selected = 0;
            *scroll = 0;
        }
        // shortcut: Space selects immediately; search a word fragment for names containing spaces.
        KeyCode::Char(' ') if !filtered.is_empty() => {
            let name = &names[filtered[*selected]];
            if checked.contains(name) {
                checked.retain(|item| item != name);
            } else {
                checked.push(name.clone());
            }
        }
        KeyCode::Char(c) if !c.is_control() && c != ' ' => {
            query.push(c);
            *selected = 0;
            *scroll = 0;
        }
        _ => {}
    }
}

pub fn step_stacks(
    terminal: &mut DefaultTerminal,
    names: &[String],
    checked: &mut Vec<String>,
    query: &mut String,
) -> Result<Nav> {
    let mut selected = 0usize;
    let mut scroll = 0usize;
    loop {
        let filtered = matches(names, query);
        selected = selected.min(filtered.len().saturating_sub(1));
        let visible = terminal.size()?.height.saturating_sub(14).max(1) as usize;
        if selected < scroll {
            scroll = selected;
        }
        if selected >= scroll + visible {
            scroll = selected + 1 - visible;
        }
        let mut buttons = Vec::new();
        let mut choices = Rect::default();
        terminal.draw(|frame| {
            buttons = screen(frame, "Choose stacks", &format!(
                "Select stack directories. Recognizable .envrc files will be migrated; custom ones are left untouched.\n\nFilter: {query}  ({} match{}, {} selected)",
                filtered.len(), if filtered.len() == 1 { "" } else { "es" }, checked.len()),
                &[("Back", false), ("Continue", true)], 0,
                "Type: filter   Up/Down: move   Space: select   Backspace: delete   Enter: continue");
            let body = sections(frame.area())[1];
            choices = Rect::new(body.x + 1, body.y + 5, body.width.saturating_sub(2), body.height.saturating_sub(7));
            let entries: Vec<String> = filtered.iter().skip(scroll).take(choices.height as usize)
                .enumerate().map(|(visible_index, index)| format!("{} [{}] {}",
                    if scroll + visible_index == selected { ">" } else { " " },
                    if checked.contains(&names[*index]) { "x" } else { " " }, names[*index])).collect();
            frame.render_widget(List::new(entries), choices);
            if filtered.is_empty() { frame.render_widget(Paragraph::new("No matching directories"), choices); }
        })?;
        let event = event::read()?;
        if let Some(index) = click(&event, &buttons) {
            return Ok(if index == 0 { Nav::Back } else { Nav::Next });
        }
        if let Event::Mouse(mouse) = event {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left)
                    if choices.contains(Position::new(mouse.column, mouse.row)) =>
                {
                    let index = mouse.row.saturating_sub(choices.y) as usize + scroll;
                    if let Some(original) = filtered.get(index) {
                        selected = index;
                        let name = &names[*original];
                        if checked.contains(name) {
                            checked.retain(|item| item != name);
                        } else {
                            checked.push(name.clone());
                        }
                    }
                }
                MouseEventKind::ScrollDown => {
                    selected = (selected + 1).min(filtered.len().saturating_sub(1))
                }
                MouseEventKind::ScrollUp => selected = selected.saturating_sub(1),
                _ => {}
            }
        }
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc => return Ok(Nav::Back),
                KeyCode::Enter => return Ok(Nav::Next),
                other => stack_key(other, names, checked, query, &mut selected, &mut scroll),
            }
        }
    }
}

pub fn step_review(
    terminal: &mut DefaultTerminal,
    title: &str,
    text: &str,
    dry_run: bool,
    first: bool,
    action: &str,
) -> Result<Nav> {
    let mut scroll = 0u16;
    let mut selected = 1usize;
    loop {
        let mut buttons = Vec::new();
        terminal.draw(|frame| {
            buttons = screen(
                frame,
                title,
                text,
                &[
                    (if first { "Cancel" } else { "Back" }, selected == 0),
                    (
                        if dry_run { "Finish preview" } else { action },
                        selected == 1,
                    ),
                ],
                scroll,
                "Up/Down: scroll   Tab/Left/Right: focus   Enter: activate   Esc: back",
            )
        })?;
        let event = event::read()?;
        if let Some(index) = click(&event, &buttons) {
            return Ok(if index == 0 { back(first) } else { Nav::Next });
        }
        if let Event::Mouse(mouse) = event {
            match mouse.kind {
                MouseEventKind::ScrollDown => scroll = scroll.saturating_add(2),
                MouseEventKind::ScrollUp => scroll = scroll.saturating_sub(2),
                _ => {}
            }
        }
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc => return Ok(back(first)),
                KeyCode::Down => scroll = scroll.saturating_add(1),
                KeyCode::Up => scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => scroll = scroll.saturating_add(10),
                KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    selected = 1 - selected
                }
                KeyCode::Enter => {
                    return Ok(if selected == 0 {
                        back(first)
                    } else {
                        Nav::Next
                    });
                }
                _ => {}
            }
        }
    }
}

pub fn notice(terminal: &mut DefaultTerminal, title: &str, message: &str) -> Result<()> {
    let mut scroll = 0u16;
    loop {
        let mut buttons = Vec::new();
        terminal.draw(|frame| {
            buttons = screen(
                frame,
                title,
                message,
                &[(
                    if title.contains("complete") {
                        "Close"
                    } else {
                        "Continue"
                    },
                    true,
                )],
                scroll,
                "Up/Down: scroll   Enter/Space: continue",
            );
        })?;
        let event = event::read()?;
        if click(&event, &buttons).is_some() {
            return Ok(());
        }
        if let Event::Mouse(mouse) = event {
            match mouse.kind {
                MouseEventKind::ScrollDown => scroll = scroll.saturating_add(2),
                MouseEventKind::ScrollUp => scroll = scroll.saturating_sub(2),
                _ => {}
            }
        }
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Down => scroll = scroll.saturating_add(1),
                KeyCode::PageDown => scroll = scroll.saturating_add(10),
                KeyCode::Up => scroll = scroll.saturating_sub(1),
                KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                _ => return Ok(()),
            }
        }
    }
}

pub fn progress(frame: &mut Frame, label: &str, done: usize, total: usize) {
    progress_with_output(frame, label, done, total, &[]);
}

pub fn progress_with_output(
    frame: &mut Frame,
    label: &str,
    done: usize,
    total: usize,
    output: &[String],
) {
    progress_frame(
        frame,
        label,
        done as f64 / total.max(1) as f64,
        &format!("Overall progress  {done}/{total}"),
        false,
        output,
    );
}

pub fn mock_progress(frame: &mut Frame, label: &str, done: usize, total: usize, fraction: f64) {
    progress_frame(
        frame,
        label,
        (done as f64 + fraction) / total.max(1) as f64,
        &format!("Mock progress  {}/{}", (done + 1).min(total), total),
        true,
        &[],
    );
}

fn progress_frame(
    frame: &mut Frame,
    label: &str,
    ratio: f64,
    title: &str,
    dry_run: bool,
    output: &[String],
) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(5),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(frame.area());
    frame.render_widget(
        Paragraph::new(if dry_run {
            " Foundry OpenCode  /  DRY-RUN SIMULATION"
        } else {
            " Foundry OpenCode  /  Working"
        })
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        areas[0],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{label}\n\n{}",
            if dry_run {
                "Mock timing only; no actions are performed."
            } else {
                "Please keep this window open."
            }
        ))
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Current step")),
        areas[1],
    );
    frame.render_widget(
        Paragraph::new(
            output
                .iter()
                .rev()
                .take(areas[2].height.saturating_sub(2) as usize)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Terminal output"),
        ),
        areas[2],
    );
    frame.render_widget(
        Gauge::default()
            .block(Block::default().title(title).borders(Borders::ALL))
            .ratio(ratio.clamp(0.0, 1.0))
            .gauge_style(Style::default().fg(Color::Cyan)),
        areas[3],
    );
    frame.render_widget(
        Paragraph::new(if dry_run {
            " Preview only: no install/removal commands, browser, or files changed."
        } else {
            " Please keep this window open until installation completes."
        })
        .style(Style::default().fg(Color::Gray)),
        areas[4],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseEvent};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn buttons_are_clickable_in_their_own_areas() {
        let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
        let mut areas = Vec::new();
        terminal
            .draw(|frame| {
                areas = screen(
                    frame,
                    "Review",
                    "No secrets here",
                    &[("No", true), ("Yes", false)],
                    0,
                    "Enter: activate",
                )
            })
            .unwrap();
        assert_eq!(areas.len(), 2);
        assert_ne!(areas[0], areas[1]);
        assert_eq!(areas[1].right(), 70); // Continue is the rightmost action.
        assert_eq!(sections(Rect::new(0, 0, 70, 20))[3].y, 19); // Help is the bottom row.
        let second = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: areas[1].x + 1,
            row: areas[1].y + 1,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(click(&second, &areas), Some(1));
    }

    #[test]
    fn progress_displays_recent_terminal_output() {
        let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
        terminal
            .draw(|frame| {
                progress_with_output(
                    frame,
                    "Indexing",
                    1,
                    2,
                    &["npm installed".into(), "indexing files".into()],
                )
            })
            .unwrap();
        let display = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(display.contains("Terminal output"));
        assert!(display.contains("indexing files"));
    }

    #[test]
    fn stack_search_is_case_insensitive_and_does_not_change_selection() {
        let names = vec!["Stack One".to_string(), "Other Stack".to_string()];
        let selected = vec!["Other Stack".to_string()];
        assert_eq!(matches(&names, "stack o"), vec![0]);
        assert_eq!(matches(&names, "other"), vec![1]);
        assert_eq!(selected, ["Other Stack"]);
    }

    #[test]
    fn space_selects_immediately_after_typing_search() {
        let names = vec!["Alpha Stack".into(), "Beta Stack".into()];
        let (mut query, mut checked, mut selected, mut scroll) = (String::new(), vec![], 0, 0);
        for key in "alpha".chars() {
            stack_key(
                KeyCode::Char(key),
                &names,
                &mut checked,
                &mut query,
                &mut selected,
                &mut scroll,
            );
        }
        stack_key(
            KeyCode::Char(' '),
            &names,
            &mut checked,
            &mut query,
            &mut selected,
            &mut scroll,
        );
        assert_eq!(query, "alpha");
        assert_eq!(checked, ["Alpha Stack"]);
        stack_key(
            KeyCode::Backspace,
            &names,
            &mut checked,
            &mut query,
            &mut selected,
            &mut scroll,
        );
        assert_eq!(checked, ["Alpha Stack"]); // Changing the filter keeps the selection.
    }

    #[test]
    fn token_button_is_keyboard_accessible_but_not_default() {
        assert_eq!(text_action(0, false, true), Nav::Next);
        assert_eq!(text_action(2, false, true), Nav::Open);
        assert_eq!(text_action(1, true, false), Nav::Cancel);
    }

    #[test]
    fn projects_suggestions_complete_existing_directories_only() {
        let home = crate::ops::backup_path(&std::env::temp_dir().join("foundry-suggestions"));
        fs::create_dir(&home).unwrap();
        fs::create_dir(home.join("Projects")).unwrap();
        fs::create_dir(home.join("Project With Space")).unwrap();
        fs::write(home.join("Pro-file"), "not a directory").unwrap();
        assert_eq!(
            directory_suggestions("~/Pro", &home),
            ["~/Project With Space", "~/Projects"]
        );
        assert_eq!(
            directory_suggestions(&format!("{}/Proj", home.display()), &home).len(),
            2
        );
        assert!(directory_suggestions("~/Missing", &home).is_empty());
        fs::remove_dir_all(home).unwrap();
    }
}
