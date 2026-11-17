//! Translate egui input into Chromium input only for the focused web surface.
use super::*;
use codisseu_core::browser::{
    BrowserId, ClipboardAction, Command as C, KeyKind, Modifiers as M, MouseButton, Surface,
};

impl CodisseuApp {
    pub(super) fn browser_surface_input(
        &mut self,
        ui: &mut egui::Ui,
        id: BrowserId,
        surface: Surface,
        rect: egui::Rect,
    ) {
        let response = ui.interact(
            rect,
            egui::Id::unique(("browser.surface", id, surface)),
            egui::Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Pane,
                ui.is_enabled(),
                if surface == Surface::Page {
                    "Página do navegador"
                } else {
                    "DevTools do navegador"
                },
            )
        });
        let modifiers = ui.input(|i| {
            cef_modifiers(
                i.modifiers,
                i.pointer.button_down(egui::PointerButton::Primary),
                i.pointer.button_down(egui::PointerButton::Middle),
                i.pointer.button_down(egui::PointerButton::Secondary),
            )
        });
        let blocked = !ui.is_enabled()
            || self.browser_input_blocked(ui.ctx())
            || self.service.browser.state != codisseu_core::browser::EngineState::Ready;
        if blocked && self.browser_ui.pointer_capture == Some((id, surface)) {
            self.browser_ui.pointer_capture = None;
            self.browser_send(C::Focus {
                id,
                surface,
                focused: false,
            });
        }
        let events = if blocked {
            Vec::new()
        } else {
            ui.input(|i| i.events.clone())
        };
        let local = |p: egui::Pos2| {
            (
                (p.x - rect.left()).round() as i32,
                (p.y - rect.top()).round() as i32,
            )
        };
        let hovered = !blocked && (response.hovered() || response.is_pointer_button_down_on());
        if hovered {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                let (x, y) = local(pos);
                if ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                    || !self.browser_ui.hovered.contains(&(id, surface))
                {
                    self.browser_send(C::MouseMove {
                        id,
                        surface,
                        x,
                        y,
                        leave: false,
                        modifiers,
                    });
                }
                self.browser_ui.hovered.insert((id, surface));
            }
        } else if self.browser_ui.hovered.remove(&(id, surface)) {
            self.browser_send(C::MouseMove {
                id,
                surface,
                x: 0,
                y: 0,
                leave: true,
                modifiers,
            });
        }
        for event in &events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } if (rect.contains(*pos) && response.contains_pointer())
                    || self.browser_ui.pointer_capture == Some((id, surface)) =>
                {
                    let button = match button {
                        egui::PointerButton::Primary => MouseButton::Left,
                        egui::PointerButton::Middle => MouseButton::Middle,
                        egui::PointerButton::Secondary => MouseButton::Right,
                        _ => continue,
                    };
                    if *pressed {
                        response.request_focus();
                        self.browser_ui.pointer_capture = Some((id, surface));
                        self.browser_send(C::Focus {
                            id,
                            surface,
                            focused: true,
                        });
                        self.browser_ui.focused = Some((id, surface));
                        let view = self.browser_ui.tabs.get(&id).map_or(0, |t| t.view);
                        if self.service.active_view() != Some(view)
                            && self.service.view(view).is_some()
                        {
                            self.send(Command::ActivateView { view });
                        }
                        self.focus_editor = false;
                    }
                    let (x, y) = local(*pos);
                    self.browser_send(C::MouseClick {
                        id,
                        surface,
                        x,
                        y,
                        button,
                        up: !*pressed,
                        clicks: if response.triple_clicked() {
                            3
                        } else if response.double_clicked() {
                            2
                        } else {
                            1
                        },
                        modifiers,
                    });
                    if !pressed {
                        self.browser_ui.pointer_capture = None;
                    }
                }
                egui::Event::MouseWheel { delta, unit, .. } if hovered => {
                    let delta = *delta
                        * match unit {
                            egui::MouseWheelUnit::Point => 1.,
                            egui::MouseWheelUnit::Line => 40.,
                            egui::MouseWheelUnit::Page => rect.height(),
                        };
                    let (x, y) = ui
                        .input(|i| i.pointer.hover_pos())
                        .map(local)
                        .unwrap_or((0, 0));
                    self.browser_send(C::MouseWheel {
                        id,
                        surface,
                        x,
                        y,
                        delta_x: delta.x.round().clamp(-16384., 16384.) as i32,
                        delta_y: delta.y.round().clamp(-16384., 16384.) as i32,
                        modifiers,
                    });
                }
                _ => {}
            }
        }
        if response.has_focus() && !blocked {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect,
                    cursor_rect: egui::Rect::from_min_size(rect.min, egui::vec2(2., 20.)),
                    should_interrupt_composition: false,
                })
            });
            let ime = events.iter().any(|e| matches!(e, egui::Event::Ime(_)));
            for event in &events {
                match event {
                    egui::Event::Text(text) if !ime => {
                        for c in text.chars() {
                            if c as u32 > 0xffff {
                                self.browser_send(C::Text {
                                    id,
                                    surface,
                                    text: c.to_string(),
                                });
                            } else {
                                self.browser_send(C::Key {
                                    id,
                                    surface,
                                    kind: KeyKind::Char,
                                    windows_key_code: c as i32,
                                    native_key_code: 0,
                                    modifiers,
                                    character: Some(c),
                                    unmodified: Some(c),
                                });
                            }
                        }
                    }
                    egui::Event::Paste(text) => {
                        if self
                            .browser_ui
                            .pending_paste
                            .is_none_or(|target| target == (id, surface))
                        {
                            self.browser_send(C::Text {
                                id,
                                surface,
                                text: text.clone(),
                            });
                            self.browser_ui.pending_paste = None;
                            self.browser_ui.paste_requested = None;
                        }
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(text)) => self.browser_send(C::Text {
                        id,
                        surface,
                        text: text.clone(),
                    }),
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text,
                        active_range_chars,
                    }) => {
                        let len = text.chars().count();
                        let cursor = active_range_chars.clone().unwrap_or(len..len);
                        let at = |n: usize| {
                            text.chars().take(n).map(char::len_utf16).sum::<usize>() as i32
                        };
                        self.browser_send(C::Composition {
                            id,
                            surface,
                            text: text.clone(),
                            selection_start: at(cursor.start),
                            selection_end: at(cursor.end),
                        });
                    }
                    egui::Event::Copy => self.browser_send(C::Clipboard {
                        id,
                        surface,
                        action: ClipboardAction::Copy,
                    }),
                    egui::Event::Cut => self.browser_send(C::Clipboard {
                        id,
                        surface,
                        action: ClipboardAction::Cut,
                    }),
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers: keys,
                        ..
                    } => {
                        // Clipboard events are already supplied by eframe; don't execute
                        // Ctrl+X/V twice through a second raw shortcut.
                        if keys.command && matches!(key, Key::C | Key::V | Key::X) {
                            continue;
                        }
                        if let Some(code) = virtual_key(*key) {
                            let mods = cef_modifiers(
                                *keys,
                                modifiers.left_button,
                                modifiers.middle_button,
                                modifiers.right_button,
                            );
                            self.browser_send(C::Key {
                                id,
                                surface,
                                kind: if *pressed {
                                    KeyKind::RawDown
                                } else {
                                    KeyKind::Up
                                },
                                windows_key_code: code,
                                native_key_code: 0,
                                modifiers: mods,
                                character: None,
                                unmodified: None,
                            });
                            if *pressed
                                && let Some(c) = match key {
                                    Key::Enter => Some('\r'),
                                    Key::Tab => Some('\t'),
                                    Key::Backspace => Some('\u{8}'),
                                    _ => None,
                                }
                            {
                                self.browser_send(C::Key {
                                    id,
                                    surface,
                                    kind: KeyKind::Char,
                                    windows_key_code: code,
                                    native_key_code: 0,
                                    modifiers: mods,
                                    character: Some(c),
                                    unmodified: Some(c),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            ui.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Text(_)
                            | egui::Event::Key { .. }
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Paste(_)
                            | egui::Event::Ime(_)
                    )
                })
            });
        }
        response.context_menu(|ui| {
            for (action, label) in [
                (ClipboardAction::Copy, "Copiar"),
                (ClipboardAction::Cut, "Recortar"),
                (ClipboardAction::SelectAll, "Selecionar tudo"),
            ] {
                if ui.button(label).clicked() {
                    self.browser_send(C::Clipboard {
                        id,
                        surface,
                        action,
                    });
                    ui.close();
                }
            }
            if ui.button("Colar").clicked() {
                self.browser_ui.pending_paste = Some((id, surface));
                self.browser_ui.paste_requested = Some(std::time::Instant::now());
                self.browser_ui.paste_canceled = false;
                response.request_focus();
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                ui.close();
            }
            ui.separator();
            if ui.button("Ferramentas do desenvolvedor").clicked() {
                self.browser_send(C::DevTools { id, open: true });
                ui.close();
            }
            if ui.button("Recarregar").clicked() {
                self.browser_send(C::Reload {
                    id,
                    ignore_cache: false,
                });
                ui.close();
            }
        });
    }
}
fn cef_modifiers(m: egui::Modifiers, left: bool, middle: bool, right: bool) -> M {
    M {
        shift: m.shift,
        ctrl: m.ctrl,
        alt: m.alt,
        meta: m.mac_cmd,
        left_button: left,
        middle_button: middle,
        right_button: right,
        ..M::default()
    }
}
fn virtual_key(key: Key) -> Option<i32> {
    Some(match key {
        Key::Backspace => 8,
        Key::Tab => 9,
        Key::Enter => 13,
        Key::Escape => 27,
        Key::Space => 32,
        Key::PageUp => 33,
        Key::PageDown => 34,
        Key::End => 35,
        Key::Home => 36,
        Key::ArrowLeft => 37,
        Key::ArrowUp => 38,
        Key::ArrowRight => 39,
        Key::ArrowDown => 40,
        Key::Insert => 45,
        Key::Delete => 46,
        Key::Num0 => 48,
        Key::Num1 => 49,
        Key::Num2 => 50,
        Key::Num3 => 51,
        Key::Num4 => 52,
        Key::Num5 => 53,
        Key::Num6 => 54,
        Key::Num7 => 55,
        Key::Num8 => 56,
        Key::Num9 => 57,
        Key::A => 65,
        Key::B => 66,
        Key::C => 67,
        Key::D => 68,
        Key::E => 69,
        Key::F => 70,
        Key::G => 71,
        Key::H => 72,
        Key::I => 73,
        Key::J => 74,
        Key::K => 75,
        Key::L => 76,
        Key::M => 77,
        Key::N => 78,
        Key::O => 79,
        Key::P => 80,
        Key::Q => 81,
        Key::R => 82,
        Key::S => 83,
        Key::T => 84,
        Key::U => 85,
        Key::V => 86,
        Key::W => 87,
        Key::X => 88,
        Key::Y => 89,
        Key::Z => 90,
        Key::F1 => 112,
        Key::F2 => 113,
        Key::F3 => 114,
        Key::F4 => 115,
        Key::F5 => 116,
        Key::F6 => 117,
        Key::F7 => 118,
        Key::F8 => 119,
        Key::F9 => 120,
        Key::F10 => 121,
        Key::F11 => 122,
        Key::F12 => 123,
        Key::Semicolon | Key::Colon => 186,
        Key::Equals | Key::Plus => 187,
        Key::Comma => 188,
        Key::Minus => 189,
        Key::Period => 190,
        Key::Slash | Key::Questionmark => 191,
        Key::Backtick => 192,
        Key::OpenBracket | Key::OpenCurlyBracket => 219,
        Key::Backslash | Key::Pipe => 220,
        Key::CloseBracket | Key::CloseCurlyBracket => 221,
        Key::Quote => 222,
        _ => return None,
    })
}
