use color_eyre::eyre::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Gauge, List, Paragraph, Wrap},
};
#[cfg(target_os = "macos")]
use std::{ffi::CString, os::macos::fs::MetadataExt, os::unix::ffi::OsStrExt};
use std::{
    fs,
    path::{Path, PathBuf},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
struct Input {
    cursor: usize, // graphemes, not bytes
}

impl Input {
    fn edit(&mut self, value: &mut String, key: KeyEvent) -> bool {
        let parts: Vec<&str> = value.graphemes(true).collect();
        self.cursor = self.cursor.min(parts.len());
        let start = parts[..self.cursor].concat().len();
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let super_key = key.modifiers.contains(KeyModifiers::SUPER);
        match key.code {
            KeyCode::Left | KeyCode::Char('b') if key.code == KeyCode::Left || (control || alt) => {
                self.cursor = self.cursor.saturating_sub(1)
            }
            KeyCode::Right | KeyCode::Char('f')
                if key.code == KeyCode::Right || (control || alt) =>
            {
                self.cursor = (self.cursor + 1).min(parts.len())
            }
            KeyCode::Home | KeyCode::Char('a')
                if key.code == KeyCode::Home || control || super_key =>
            {
                self.cursor = 0
            }
            KeyCode::End | KeyCode::Char('e')
                if key.code == KeyCode::End || control || super_key =>
            {
                self.cursor = parts.len()
            }
            KeyCode::Char('u') if control => {
                value.drain(..start);
                self.cursor = 0;
            }
            KeyCode::Char('k') if control => {
                value.truncate(start);
            }
            KeyCode::Backspace if super_key || control => {
                value.drain(..start);
                self.cursor = 0;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                value.drain(start - parts[self.cursor - 1].len()..start);
                self.cursor -= 1;
            }
            KeyCode::Delete if self.cursor < parts.len() => {
                value.drain(start..start + parts[self.cursor].len());
            }
            KeyCode::Char(c) if !c.is_control() && !control && !alt && !super_key => {
                value.insert(start, c);
                self.cursor = value[..start + c.len_utf8()].graphemes(true).count();
            }
            _ => return false,
        }
        true
    }

    fn view(&self, value: &str, width: usize, secret: bool) -> (String, usize, usize) {
        let parts: Vec<&str> = value.graphemes(true).collect();
        let cursor = self.cursor.min(parts.len());
        let width_of = |part: &str| if secret { 1 } else { part.width().max(1) };
        let mut start = 0;
        while start < cursor
            && parts[start..cursor]
                .iter()
                .map(|p| width_of(p))
                .sum::<usize>()
                >= width
        {
            start += 1;
        }
        let mut end = start;
        let mut used = 0;
        while end < parts.len() && used + width_of(parts[end]) <= width {
            used += width_of(parts[end]);
            end += 1;
        }
        let text = if secret {
            "•".repeat(end - start)
        } else {
            parts[start..end].concat()
        };
        let column = parts[start..cursor].iter().map(|p| width_of(p)).sum();
        (text, start, column)
    }

    fn click(&mut self, value: &str, start: usize, column: usize, secret: bool) {
        self.cursor = start;
        let mut used = 0;
        for part in value.graphemes(true).skip(start) {
            let width = if secret { 1 } else { part.width().max(1) };
            if used + width > column {
                break;
            }
            used += width;
            self.cursor += 1;
        }
    }
}

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

fn directory_suggestions(
    input: &str,
    home: &Path,
    base: &Path,
    warning: &mut String,
) -> Vec<String> {
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
        base.to_path_buf()
    } else {
        base.join(prefix)
    };
    let entries = match fs::read_dir(&parent) {
        Ok(entries) => entries,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                *warning = format!("Cannot browse {}: {error}", parent.display());
            }
            return Vec::new();
        }
    };
    let mut suggestions: Vec<_> = entries
        .filter_map(|entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    *warning = format!("Cannot read {}: {error}", parent.display());
                    return None;
                }
            };
            let name = entry.file_name().into_string().ok()?;
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    *warning = format!("Cannot inspect {}: {error}", entry.path().display());
                    return None;
                }
            };
            if !kind.is_dir()
                || name.chars().any(char::is_control)
                || !name.to_lowercase().starts_with(&fragment.to_lowercase())
            {
                return None;
            }
            match hidden(&entry) {
                Ok(true)
                    if !(fragment.starts_with('.') && name.starts_with('.'))
                        && name != fragment =>
                {
                    return None;
                }
                Err(error) => {
                    *warning = format!("Cannot inspect {}: {error}", entry.path().display());
                    return None;
                }
                _ => {}
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
pub enum Field {
    Plain,
    Token,
}

pub fn step_text(
    terminal: &mut DefaultTerminal,
    title: &str,
    prompt: &str,
    value: &mut String,
    first: bool,
    kind: Field,
) -> Result<Nav> {
    let secret = matches!(kind, Field::Token);
    let open = matches!(kind, Field::Token);
    let mut focus = 0usize; // input, Back, optional Open, Continue
    let mut input = Input {
        cursor: value.graphemes(true).count(),
    };
    let mut scroll = 0u16;
    loop {
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
        let mut view_start = 0;
        terminal.draw(|frame| {
            buttons = screen(
                frame,
                title,
                prompt,
                &actions,
                scroll,
                "Arrows: cursor  Ctrl+U/Cmd+Delete: clear  Tab: focus  Enter: continue  Esc: back",
            );
            let body = sections(frame.area())[1];
            field = Rect::new(
                body.x + 1,
                body.bottom().saturating_sub(4),
                body.width.saturating_sub(2),
                3,
            );
            let width = field.width.saturating_sub(2) as usize;
            let (visible, start, column) = input.view(value, width, secret);
            view_start = start;
            frame.render_widget(
                Paragraph::new(visible).block(
                    Block::default()
                        .title(if secret { "Token (masked)" } else { "Input" })
                        .borders(Borders::ALL),
                ),
                field,
            );
            if focus == 0 && width > 0 {
                frame.set_cursor_position(Position::new(
                    field.x + 1 + (column.min(width - 1) as u16),
                    field.y + 1,
                ));
            }
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
            && field.contains(Position::new(mouse.column, mouse.row))
        {
            focus = 0;
            input.click(
                value,
                view_start,
                mouse.column.saturating_sub(field.x + 1) as usize,
                secret,
            );
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
                KeyCode::Tab => focus = (focus + 1) % (actions.len() + 1),
                KeyCode::BackTab => focus = (focus + actions.len()) % (actions.len() + 1),
                KeyCode::PageDown => scroll = scroll.saturating_add(3),
                KeyCode::PageUp => scroll = scroll.saturating_sub(3),
                KeyCode::Down => scroll = scroll.saturating_add(1),
                KeyCode::Up => scroll = scroll.saturating_sub(1),
                _ if focus == 0 => {
                    input.edit(value, key);
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

fn matching_directories<'a>(directories: &'a [PathBuf], query: &str) -> Vec<&'a PathBuf> {
    let query = query.to_lowercase();
    directories
        .iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(&query))
        })
        .collect()
}

fn path_query(query: &str) -> bool {
    query.contains('/') || query.starts_with('.') || query == "~"
}

fn resolve_path(query: &str, home: &Path, current: &Path) -> PathBuf {
    if query == "~" {
        home.to_path_buf()
    } else if let Some(rest) = query.strip_prefix("~/") {
        home.join(rest)
    } else if Path::new(query).is_absolute() {
        PathBuf::from(query)
    } else {
        current.join(query)
    }
}

#[cfg(target_os = "macos")]
fn finder_invisible(data: &[u8]) -> std::io::Result<bool> {
    if data.len() != 32 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid FinderInfo attribute",
        ));
    }
    Ok(u16::from_be_bytes([data[8], data[9]]) & 0x4000 != 0)
}

#[cfg(target_os = "macos")]
fn macos_invisible(flags: u32, finder_info: Option<&[u8]>) -> std::io::Result<bool> {
    if flags & libc::UF_HIDDEN != 0 {
        return Ok(true);
    }
    finder_info.map(finder_invisible).unwrap_or(Ok(false))
}

fn hidden(entry: &fs::DirEntry) -> std::io::Result<bool> {
    #[cfg(target_os = "macos")]
    {
        if entry.file_name().as_os_str().as_bytes().starts_with(b".") {
            return Ok(true);
        }
        let flags = entry.metadata()?.st_flags();
        if flags & libc::UF_HIDDEN != 0 {
            return Ok(true);
        }
        let path = CString::new(entry.path().as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid directory path")
        })?;
        let mut data = [0u8; 32];
        let size = unsafe {
            libc::getxattr(
                path.as_ptr(),
                c"com.apple.FinderInfo".as_ptr(),
                data.as_mut_ptr().cast(),
                data.len(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        if size < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ENOATTR | libc::ENOTSUP)) {
                return Ok(false);
            }
            return Err(error);
        }
        macos_invisible(flags, Some(&data[..size as usize]))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(entry.file_name().to_string_lossy().starts_with('.'))
    }
}

fn directories(path: &Path, warning: &mut String) -> std::io::Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                *warning = format!("Cannot read {}: {error}", path.display());
                continue;
            }
        };
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => match hidden(&entry) {
                Ok(false) => entries.push(entry.path()),
                Ok(true) => {}
                Err(error) => {
                    *warning = format!("Cannot inspect {}: {error}", entry.path().display())
                }
            },
            Ok(_) => {}
            Err(error) => *warning = format!("Cannot inspect {}: {error}", entry.path().display()),
        }
    }
    entries.sort_by_key(|path| path.file_name().unwrap().to_string_lossy().to_lowercase());
    Ok(entries)
}

fn browse(current: &mut PathBuf, path: &Path) -> std::io::Result<()> {
    // Check access before updating state so a failed open never strands the explorer.
    fs::read_dir(path)?;
    *current = path.canonicalize()?;
    Ok(())
}

fn move_tree_selection(tree: &[(PathBuf, usize)], cursor: &mut PathBuf, delta: isize) {
    let index = tree
        .iter()
        .position(|(path, _)| path == cursor)
        .unwrap_or(0);
    let next = index
        .saturating_add_signed(delta)
        .min(tree.len().saturating_sub(1));
    if let Some((path, _)) = tree.get(next) {
        *cursor = path.clone();
    }
}

fn explorer_tab(focus: usize) -> usize {
    1 - focus
}

fn toggle_folder(checked: &mut Vec<PathBuf>, path: &Path) {
    if checked.iter().any(|item| item == path) {
        checked.retain(|item| item != path);
    } else {
        checked.push(path.to_path_buf());
    }
}

fn append_tree(
    path: &Path,
    depth: usize,
    current: &Path,
    expanded: &std::collections::BTreeSet<PathBuf>,
    rows: &mut Vec<(PathBuf, usize)>,
    warning: &mut String,
) {
    rows.push((path.to_path_buf(), depth));
    if !expanded.contains(path) {
        return;
    }
    match directories(path, warning) {
        Ok(mut children) => {
            if let Some(hidden_parent) = current
                .ancestors()
                .find(|ancestor| ancestor.parent() == Some(path))
                && !children.iter().any(|child| child == hidden_parent)
            {
                children.push(hidden_parent.to_path_buf());
                children.sort_by_key(|child| {
                    child.file_name().unwrap().to_string_lossy().to_lowercase()
                });
            }
            for child in children {
                append_tree(&child, depth + 1, current, expanded, rows, warning);
            }
        }
        Err(error) => *warning = format!("Cannot browse {}: {error}", path.display()),
    }
}

fn filter_tree(rows: &mut Vec<(PathBuf, usize)>, current: &Path, query: &str) {
    if query.is_empty() {
        return;
    }
    let paths: Vec<PathBuf> = rows.iter().map(|(path, _)| path.clone()).collect();
    let mut keep = std::collections::BTreeSet::new();
    for (path, _) in rows.iter() {
        if current.starts_with(path) {
            keep.insert(path.clone());
        }
    }
    for path in matching_directories(&paths, query) {
        keep.extend(path.ancestors().map(Path::to_path_buf));
    }
    rows.retain(|(path, _)| keep.contains(path));
}

fn tree_rows(
    current: &Path,
    query: &str,
    home: &Path,
    expanded: &std::collections::BTreeSet<PathBuf>,
    warning: &mut String,
) -> (Vec<(PathBuf, usize)>, Vec<PathBuf>) {
    if path_query(query) {
        let suggestions: Vec<PathBuf> = directory_suggestions(query, home, current, warning)
            .iter()
            .map(|path| resolve_path(path, home, current))
            .collect();
        let mut paths = std::collections::BTreeSet::new();
        for path in std::iter::once(current).chain(suggestions.iter().map(PathBuf::as_path)) {
            paths.extend(path.ancestors().map(Path::to_path_buf));
        }
        return (
            paths
                .into_iter()
                .map(|path| {
                    let depth = path.components().count().saturating_sub(1);
                    (path, depth)
                })
                .collect(),
            suggestions,
        );
    }
    let mut rows = Vec::new();
    append_tree(Path::new("/"), 0, current, expanded, &mut rows, warning);
    filter_tree(&mut rows, current, query);
    (rows, Vec::new())
}

pub fn step_explorer(
    terminal: &mut DefaultTerminal,
    home: &Path,
    current: &mut PathBuf,
    checked: &mut Vec<PathBuf>,
) -> Result<Nav> {
    explorer(terminal, home, current, checked, "stack", false)
}

pub fn step_initial_explorer(
    terminal: &mut DefaultTerminal,
    home: &Path,
    current: &mut PathBuf,
    checked: &mut Vec<PathBuf>,
) -> Result<Nav> {
    explorer(terminal, home, current, checked, "stack", true)
}

pub fn step_project_explorer(
    terminal: &mut DefaultTerminal,
    home: &Path,
    current: &mut PathBuf,
    checked: &mut Vec<PathBuf>,
) -> Result<Nav> {
    explorer(terminal, home, current, checked, "RAG project", false)
}

fn explorer(
    terminal: &mut DefaultTerminal,
    home: &Path,
    current: &mut PathBuf,
    checked: &mut Vec<PathBuf>,
    kind: &str,
    first: bool,
) -> Result<Nav> {
    let mut scroll = 0usize;
    let mut focus = 0usize; // tree, search
    let mut query = String::new();
    let mut search = Input::default();
    let mut suggestion_selected = false;
    let mut tree_cursor = current.clone();
    let mut expanded: std::collections::BTreeSet<PathBuf> =
        current.ancestors().map(Path::to_path_buf).collect();
    let mut warning = String::new();
    loop {
        loop {
            match directories(current, &mut warning) {
                Ok(_) => break,
                Err(error) => {
                    warning = format!("Cannot browse {}: {error}", current.display());
                    let Some(parent) = current.parent() else {
                        return Err(error.into());
                    };
                    *current = parent.to_path_buf();
                    tree_cursor = current.clone();
                }
            }
        }
        let is_path = path_query(&query);
        let (tree, suggestions) = tree_rows(current, &query, home, &expanded, &mut warning);
        if !tree.iter().any(|(path, _)| path == &tree_cursor) {
            tree_cursor = current.clone();
            suggestion_selected = false;
        }
        let tree_selected = tree
            .iter()
            .position(|(path, _)| path == &tree_cursor)
            .unwrap_or_else(|| {
                tree.iter()
                    .position(|(path, _)| path == current)
                    .unwrap_or(0)
            });
        let scroll_selected = if focus == 1 && is_path && !suggestion_selected {
            suggestions
                .first()
                .and_then(|first| tree.iter().position(|(path, _)| path == first))
                .unwrap_or(tree_selected)
        } else {
            tree_selected
        };
        let mut buttons = Vec::new();
        let mut tree_area = Rect::default();
        let mut search_area = Rect::default();
        let mut search_start = 0;
        let search_prefix = "Search / path: ";
        let visible = terminal.size()?.height.saturating_sub(15).max(1) as usize;
        if scroll_selected < scroll {
            scroll = scroll_selected;
        }
        if scroll_selected >= scroll + visible {
            scroll = scroll_selected.saturating_sub(visible / 2);
        }
        terminal.draw(|frame| {
            buttons = screen(frame, &format!("Choose {kind} directories"), &format!(
                "Browsing {}\n{} marked as {kind} folders. Hidden folders omitted; type a full path to open one.",
                current.display(), checked.len()),
                &[(if first { "Cancel" } else { "Back" }, false), ("Continue", true)], 0,
                "Tab: tree/search  S: search  /: path  Enter/Right: expand  Left: collapse  Space: mark  C: continue");
            let body = sections(frame.area())[1];
            search_area = Rect::new(body.x + 1, body.y + 3, body.width.saturating_sub(2), 1);
            frame.render_widget(Paragraph::new(warning.as_str()).style(Style::default().fg(Color::Yellow)),
                Rect::new(body.x + 1, body.y + 4, body.width.saturating_sub(2), 1));
            tree_area = Rect::new(body.x + 1, body.y + 5, body.width.saturating_sub(2), body.height.saturating_sub(6));
            let width = search_area.width.saturating_sub(search_prefix.len() as u16) as usize;
            let (shown, start, column) = search.view(&query, width, false);
            search_start = start;
            frame.render_widget(Paragraph::new(format!("{search_prefix}{shown}")), search_area);
            if focus == 1 && search_area.width > 0 {
                let col = (search_prefix.len() + column).min(search_area.width.saturating_sub(1) as usize);
                frame.set_cursor_position(Position::new(search_area.x + col as u16, search_area.y));
            }
            let tree_items = tree.iter().skip(scroll).take(tree_area.height.saturating_sub(2) as usize).enumerate().map(|(i, (path, depth))| {
                let name = path.file_name().map(|s| format!("{}/", s.to_string_lossy())).unwrap_or_else(|| "/".into());
                format!("{}{}{} [{}] {name}", if i + scroll == tree_selected && (focus == 0 || suggestion_selected) { ">" } else { " " },
                    "  ".repeat((*depth).min(12)), if expanded.contains(path) || (is_path && tree.iter().any(|(child, _)| child != path && child.starts_with(path))) { "v" } else { ">" }, if checked.contains(path) { "x" } else { " " })
            }).collect::<Vec<_>>();
            frame.render_widget(List::new(tree_items).block(Block::default().title(if is_path { "Path matches" } else { "Directories" }).borders(Borders::ALL)), tree_area);
        })?;
        let event = event::read()?;
        let button = click(&event, &buttons);
        if button == Some(0) {
            return Ok(back(first));
        }
        if button == Some(1) {
            return Ok(Nav::Next);
        }
        let mut target = None;
        if let Event::Mouse(mouse) = &event {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && search_area.contains(Position::new(mouse.column, mouse.row))
            {
                focus = 1;
                suggestion_selected = false;
                search.click(
                    &query,
                    search_start,
                    mouse
                        .column
                        .saturating_sub(search_area.x + search_prefix.len() as u16)
                        as usize,
                    false,
                );
            }
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && tree_area.contains(Position::new(mouse.column, mouse.row))
                && let Some((path, depth)) =
                    tree.get(mouse.row.saturating_sub(tree_area.y + 1) as usize + scroll)
            {
                tree_cursor = path.clone();
                focus = 0;
                let indent = ((*depth).min(12) * 2) as u16;
                let arrow = tree_area.x + 2 + indent;
                let checkbox = tree_area.x + 4 + indent;
                if (checkbox..checkbox + 3).contains(&mouse.column) {
                    if path == home || path == Path::new("/") {
                        warning = format!(
                            "Home and the filesystem root cannot be marked as {kind} folders"
                        );
                    } else {
                        toggle_folder(checked, path);
                        warning.clear();
                    }
                } else if mouse.column == arrow {
                    target = Some((path.clone(), true));
                }
            }
            if mouse.kind == MouseEventKind::ScrollDown {
                move_tree_selection(&tree, &mut tree_cursor, 1);
                suggestion_selected = is_path;
            }
            if mouse.kind == MouseEventKind::ScrollUp {
                move_tree_selection(&tree, &mut tree_cursor, -1);
                suggestion_selected = is_path;
            }
        }
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc if focus == 1 => {
                    query.clear();
                    focus = 0;
                    scroll = 0;
                    suggestion_selected = false;
                    warning.clear();
                }
                KeyCode::Esc => return Ok(back(first)),
                KeyCode::Tab | KeyCode::BackTab => focus = explorer_tab(focus),
                KeyCode::Char('s' | 'S') if focus == 0 => {
                    focus = 1;
                    search.cursor = query.graphemes(true).count();
                }
                KeyCode::Char('/') if focus == 0 => {
                    focus = 1;
                    query = "/".into();
                    search.cursor = 1;
                    scroll = 0;
                    suggestion_selected = false;
                }
                KeyCode::Char('c') | KeyCode::Char('C')
                    if focus == 0
                        && key
                            .modifiers
                            .intersection(
                                KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                            )
                            .is_empty() =>
                {
                    return Ok(Nav::Next);
                }
                KeyCode::Down if focus == 0 => move_tree_selection(&tree, &mut tree_cursor, 1),
                KeyCode::Up if focus == 0 => move_tree_selection(&tree, &mut tree_cursor, -1),
                KeyCode::Down if focus == 1 && is_path => {
                    if let Some(index) = suggestions.iter().position(|path| path == &tree_cursor) {
                        tree_cursor = suggestions[(index + 1).min(suggestions.len() - 1)].clone();
                    } else if let Some(first) = suggestions.first() {
                        tree_cursor = first.clone();
                    }
                    suggestion_selected = suggestions.contains(&tree_cursor);
                }
                KeyCode::Up if focus == 1 && is_path => {
                    if let Some(index) = suggestions.iter().position(|path| path == &tree_cursor) {
                        tree_cursor = suggestions[index.saturating_sub(1)].clone();
                    } else if let Some(first) = suggestions.first() {
                        tree_cursor = first.clone();
                    }
                    suggestion_selected = suggestions.contains(&tree_cursor);
                }
                KeyCode::Enter if focus == 1 && is_path => {
                    let path = if suggestion_selected && suggestions.contains(&tree_cursor) {
                        tree_cursor.clone()
                    } else {
                        resolve_path(&query, home, current)
                    };
                    target = Some((path, false));
                }
                KeyCode::Enter if focus == 1 => focus = 0,
                KeyCode::Left | KeyCode::Backspace
                    if focus == 0
                        && expanded.contains(&tree_cursor)
                        && tree_cursor != Path::new("/") =>
                {
                    target = Some((tree_cursor.clone(), true));
                }
                KeyCode::Left | KeyCode::Backspace if focus == 0 => {
                    if let Some(parent) = tree_cursor.parent() {
                        tree_cursor = parent.to_path_buf();
                    }
                }
                KeyCode::Right if focus == 0 && expanded.contains(&tree_cursor) => {
                    if let Some((child, _)) = tree
                        .get(tree_selected + 1)
                        .filter(|(_, depth)| *depth > tree[tree_selected].1)
                    {
                        tree_cursor = child.clone();
                    }
                }
                KeyCode::Enter if focus == 0 => target = Some((tree_cursor.clone(), true)),
                KeyCode::Right if focus == 0 => target = Some((tree_cursor.clone(), false)),
                KeyCode::Char(' ') if focus == 0 => {
                    if tree_cursor == home || tree_cursor == Path::new("/") {
                        warning = format!(
                            "Home and the filesystem root cannot be marked as {kind} folders"
                        );
                    } else {
                        toggle_folder(checked, &tree_cursor);
                        warning.clear();
                    }
                }
                _ if focus == 1 && search.edit(&mut query, key) => {
                    scroll = 0;
                    suggestion_selected = false;
                    warning.clear();
                }
                _ => {}
            }
        }
        if let Some((path, toggle)) = target {
            if toggle && expanded.contains(&path) {
                if path != Path::new("/") {
                    expanded.remove(&path);
                    if current.starts_with(&path) {
                        *current = path.parent().unwrap().to_path_buf();
                    }
                    query.clear();
                    search.cursor = 0;
                    suggestion_selected = false;
                    focus = 0;
                    warning.clear();
                }
            } else {
                match browse(current, &path) {
                    Ok(()) => {
                        expanded.extend(current.ancestors().map(Path::to_path_buf));
                        tree_cursor = current.clone();
                        scroll = 0;
                        query.clear();
                        search.cursor = 0;
                        suggestion_selected = false;
                        focus = 0;
                        warning.clear();
                    }
                    Err(error) => warning = format!("Cannot open {}: {error}", path.display()),
                }
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
    fn input_edits_at_cursor_and_keeps_tokens_masked() {
        let key = |code, modifiers| KeyEvent::new(code, modifiers);
        let mut value = "a👩‍💻c".to_string();
        let mut input = Input { cursor: 2 };
        input.edit(&mut value, key(KeyCode::Left, KeyModifiers::NONE));
        input.edit(&mut value, key(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(value, "ab👩‍💻c");
        input.edit(&mut value, key(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(value, "abc");
        input.click(&value, 0, 2, false);
        input.edit(&mut value, key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(value, "ac");
        input.edit(&mut value, key(KeyCode::Backspace, KeyModifiers::SUPER));
        assert_eq!(value, "c");
        input.edit(&mut value, key(KeyCode::End, KeyModifiers::NONE));
        input.edit(&mut value, key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(value.is_empty());
        let (shown, _, _) = input.view("sensitive", 20, true);
        assert_eq!(shown, "•••••••••");
    }

    #[test]
    fn long_input_scrolls_without_losing_cursor_position() {
        let mut input = Input { cursor: 10 };
        let (text, start, column) = input.view("0123456789", 5, false);
        assert_eq!((text.as_str(), start, column), ("6789", 6, 4));
        input.click("0123456789", start, 2, false);
        assert_eq!(input.cursor, 8);
    }

    #[test]
    fn explorer_search_filters_only_the_current_directory_list() {
        let paths = [
            PathBuf::from("/project/Alpha"),
            PathBuf::from("/project/Other"),
        ];
        assert_eq!(matching_directories(&paths, "alp"), [&paths[0]]);
        assert_eq!(matching_directories(&paths, ""), [&paths[0], &paths[1]]);
        assert!(matching_directories(&[PathBuf::from("/")], "missing").is_empty());
    }

    #[test]
    fn tab_switches_explorer_panes_without_completing_a_path() {
        let home = crate::ops::backup_path(&std::env::temp_dir().join("explorer-tab"));
        fs::create_dir_all(home.join("Projects")).unwrap();
        let query = "./Pro".to_string();
        assert_eq!(
            directory_suggestions(&query, &home, &home, &mut String::new()),
            ["./Projects"]
        );
        assert_eq!(explorer_tab(1), 0); // search -> tree
        assert_eq!(explorer_tab(0), 1); // tree -> search (also Shift+Tab)
        assert_eq!(query, "./Pro");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_single_tree_expands_and_collapses_without_losing_marks() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("explorer-tree"));
        fs::create_dir_all(root.join("Alpha/Child")).unwrap();
        fs::create_dir(root.join("Beta")).unwrap();
        let alpha = root.join("Alpha");
        let child = alpha.join("Child");
        let mut expanded = std::collections::BTreeSet::from([root.clone()]);
        let mut rows = Vec::new();
        let mut warning = String::new();
        let mut checked = vec![child.clone()];
        append_tree(&root, 0, &root, &expanded, &mut rows, &mut warning);
        assert_eq!(
            rows.iter().map(|(path, _)| path).collect::<Vec<_>>(),
            [&root, &alpha, &root.join("Beta")]
        );
        let mut cursor = root.clone();
        move_tree_selection(&rows, &mut cursor, 1);
        assert_eq!(cursor, alpha); // highlighting does not open the child
        assert!(!rows.iter().any(|(path, _)| path == &child));
        expanded.insert(alpha.clone());
        rows.clear();
        append_tree(&root, 0, &root, &expanded, &mut rows, &mut warning);
        assert!(
            rows.iter()
                .any(|(path, depth)| path == &child && *depth == 2)
        );
        filter_tree(&mut rows, &root, "Child");
        assert_eq!(
            rows.iter().map(|(path, _)| path).collect::<Vec<_>>(),
            [&root, &alpha, &child]
        );
        expanded.remove(&alpha);
        rows.clear();
        append_tree(&root, 0, &root, &expanded, &mut rows, &mut warning);
        assert!(!rows.iter().any(|(path, _)| path == &child));
        assert_eq!(checked.len(), 1);
        assert_eq!(checked[0], child);
        toggle_folder(&mut checked, &child);
        assert!(checked.is_empty());
        assert!(warning.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hidden_folders_are_omitted_but_explicit_paths_and_marks_remain() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("explorer-hidden"));
        fs::create_dir_all(root.join(".private")).unwrap();
        fs::create_dir(root.join(".other")).unwrap();
        fs::create_dir(root.join("Visible")).unwrap();
        let root = root.canonicalize().unwrap();
        let mut warning = String::new();
        let listed = directories(&root, &mut warning).unwrap();
        assert_eq!(listed, [root.join("Visible")]);
        assert!(
            directory_suggestions("~/.", &root, &root, &mut warning).contains(&"~/.private".into())
        );
        assert!(directory_suggestions("~/p", &root, &root, &mut warning).is_empty());
        let hidden_path = resolve_path("~/.private", &root, &root);
        let mut current = root.clone();
        browse(&mut current, &hidden_path).unwrap();
        let mut rows = Vec::new();
        append_tree(
            &root,
            0,
            &current,
            &std::collections::BTreeSet::from([root.clone(), current.clone()]),
            &mut rows,
            &mut warning,
        );
        assert!(rows.iter().any(|(path, _)| path == &current));
        assert!(!rows.iter().any(|(path, _)| path == &root.join(".other")));
        let checked = vec![current.clone()];
        assert_eq!(checked, [hidden_path.canonicalize().unwrap()]);
        assert!(warning.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_and_bsd_invisible_flags_are_checked_independently() {
        let mut info = [0u8; 32];
        assert!(!macos_invisible(0, None).unwrap());
        assert!(macos_invisible(libc::UF_HIDDEN, None).unwrap());
        info[8..10].copy_from_slice(&0x0400u16.to_be_bytes()); // unrelated Finder flag
        assert!(!macos_invisible(0, Some(&info)).unwrap());
        info[8..10].copy_from_slice(&0x4000u16.to_be_bytes());
        assert!(macos_invisible(0, Some(&info)).unwrap());
        assert!(macos_invisible(0, Some(&info[..8])).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_invisible_attribute_is_hidden_from_tree_but_exact_path_works() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("explorer-finder"));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let finder = root.join("FinderInvisible");
        let flagged = root.join("FlagInvisible");
        fs::create_dir(&finder).unwrap();
        fs::create_dir(&flagged).unwrap();
        let finder_path = CString::new(finder.as_os_str().as_bytes()).unwrap();
        let flag_path = CString::new(flagged.as_os_str().as_bytes()).unwrap();
        let mut info = [0u8; 32];
        info[8..10].copy_from_slice(&0x4000u16.to_be_bytes());
        assert_eq!(
            unsafe {
                libc::setxattr(
                    finder_path.as_ptr(),
                    c"com.apple.FinderInfo".as_ptr(),
                    info.as_ptr().cast(),
                    info.len(),
                    0,
                    0,
                )
            },
            0
        );
        let flags_set = unsafe { libc::chflags(flag_path.as_ptr(), libc::UF_HIDDEN) } == 0;
        let mut warning = String::new();
        let listed = directories(&root, &mut warning).unwrap();
        assert!(!listed.contains(&finder));
        if flags_set {
            assert!(!listed.contains(&flagged));
        }
        assert!(directory_suggestions("~/Fin", &root, &root, &mut warning).is_empty());
        assert_eq!(
            directory_suggestions("~/FinderInvisible", &root, &root, &mut warning),
            ["~/FinderInvisible"]
        );
        let mut current = root.clone();
        browse(&mut current, &finder).unwrap();
        assert_eq!(current, finder.canonicalize().unwrap());
        let mut rows = Vec::new();
        let expanded = std::collections::BTreeSet::from([root.clone(), current.clone()]);
        append_tree(&root, 0, &current, &expanded, &mut rows, &mut warning);
        assert!(rows.iter().any(|(path, _)| path == &current));
        if flags_set {
            assert!(!rows.iter().any(|(path, _)| path == &flagged));
        }
        assert!(warning.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tree_highlight_does_not_open_or_change_marks() {
        let home = PathBuf::from("/home/test");
        let child = home.join("Projects");
        let tree = vec![(home.clone(), 0), (child.clone(), 1)];
        let current = home.clone();
        let mut cursor = home.clone();
        let mut checked = Vec::new();
        move_tree_selection(&tree, &mut cursor, 1);
        assert_eq!(cursor, child);
        assert_eq!(current, home);
        assert!(checked.is_empty());
        toggle_folder(&mut checked, &cursor);
        assert_eq!(checked, [child]);
        assert_eq!(current, home);
        move_tree_selection(&tree, &mut cursor, -1);
        assert_eq!(cursor, current);
        assert_eq!(current, home);
        assert_eq!(checked.len(), 1);
    }

    #[test]
    fn paths_complete_relative_to_the_current_folder_and_open_only_if_accessible() {
        let home = crate::ops::backup_path(&std::env::temp_dir().join("explorer-paths"));
        let current_dir = home.join("Current");
        fs::create_dir_all(current_dir.join("Project With Space")).unwrap();
        fs::create_dir(home.join("Other")).unwrap();
        assert!(path_query("~/Ot"));
        assert!(path_query("../Other"));
        assert!(!path_query("Project"));
        assert_eq!(
            directory_suggestions("./Proj", &home, &current_dir, &mut String::new()),
            ["./Project With Space"]
        );
        assert_eq!(
            directory_suggestions("~/Ot", &home, &current_dir, &mut String::new()),
            ["~/Other"]
        );
        let (rows, suggested) = tree_rows(
            &current_dir,
            "~/Ot",
            &home,
            &Default::default(),
            &mut String::new(),
        );
        assert_eq!(suggested, [home.join("Other")]);
        assert!(rows.iter().any(|(path, _)| path == &home.join("Other")));
        assert_eq!(
            directory_suggestions("../Ot", &home, &current_dir, &mut String::new()),
            ["../Other"]
        );
        assert_eq!(
            directory_suggestions(
                &format!("{}/Ot", home.display()),
                &home,
                &current_dir,
                &mut String::new()
            ),
            [format!("{}/Other", home.display())]
        );
        assert_eq!(
            resolve_path("../Other", &home, &current_dir),
            current_dir.join("../Other")
        );
        let mut browsing = current_dir.clone();
        let missing = resolve_path("~/missing", &home, &browsing);
        assert!(browse(&mut browsing, &missing).is_err());
        assert_eq!(browsing, current_dir);
        browse(&mut browsing, &resolve_path("~/Other", &home, &current_dir)).unwrap();
        assert_eq!(browsing, home.join("Other").canonicalize().unwrap());
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_does_not_replace_the_current_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::ops::backup_path(&std::env::temp_dir().join("explorer-permissions"));
        let locked = root.join("private");
        fs::create_dir_all(&locked).unwrap();
        let mut current = root.clone();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&locked).is_err() {
            // root-run test environments may bypass permissions
            assert_eq!(
                browse(&mut current, &locked).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert_eq!(current, root);
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

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
    fn token_button_is_keyboard_accessible_but_not_default() {
        assert_eq!(text_action(0, false, true), Nav::Next);
        assert_eq!(text_action(2, false, true), Nav::Open);
        assert_eq!(text_action(1, true, false), Nav::Cancel);
        assert_eq!(text_action(1, false, false), Nav::Back);
    }

    #[test]
    fn projects_suggestions_complete_existing_directories_only() {
        let home = crate::ops::backup_path(&std::env::temp_dir().join("foundry-suggestions"));
        fs::create_dir(&home).unwrap();
        fs::create_dir(home.join("Projects")).unwrap();
        fs::create_dir(home.join("Project With Space")).unwrap();
        fs::write(home.join("Pro-file"), "not a directory").unwrap();
        assert_eq!(
            directory_suggestions("~/Pro", &home, &home, &mut String::new()),
            ["~/Project With Space", "~/Projects"]
        );
        assert_eq!(
            directory_suggestions(
                &format!("{}/Proj", home.display()),
                &home,
                &home,
                &mut String::new()
            )
            .len(),
            2
        );
        assert!(directory_suggestions("~/Missing", &home, &home, &mut String::new()).is_empty());
        fs::remove_dir_all(home).unwrap();
    }
}
