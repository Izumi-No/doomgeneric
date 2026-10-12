//! Menu state machine, port of the logic core of `m_menu.c`.
//!
//! The menus are data (item lists) plus a cursor; input moves the cursor and
//! the menu emits [`MenuAction`]s for the engine to apply (start a game,
//! change a volume, quit...). Drawing and key-mapping are host concerns,
//! which keeps this module free of IO and screens.

use alloc::vec::Vec;

use crate::s_sound::{DEFAULT_MUSIC_VOLUME, DEFAULT_SFX_VOLUME};

/// `menuitem_t` kinds (the ones the core can act on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItemKind {
    /// A branch into another menu.
    Submenu(MenuId),
    /// Start a new game at this skill.
    Skill(i32),
    /// Start an episode.
    Episode(i32),
    /// End the game.
    Quit,
    /// No-op (vanilla draws a gap).
    Separator,
}

/// `menuitem_t`.
#[derive(Clone, Copy, Debug)]
pub struct MenuItem {
    pub label: &'static str,
    pub kind: MenuItemKind,
}

/// `menu_t` ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
    None,
    Main,
    NewGame,
    Episode,
    Options,
    Sound,
    Quit,
}

/// The static menus (shortened to the items that drive core state; the
/// vanilla screens/mouse/touch extras are host additions).
fn items(id: MenuId) -> &'static [MenuItem] {
    match id {
        MenuId::Main => &[
            MenuItem { label: "New Game", kind: MenuItemKind::Submenu(MenuId::NewGame) },
            MenuItem { label: "Options", kind: MenuItemKind::Submenu(MenuId::Options) },
            MenuItem { label: "Quit DOOM", kind: MenuItemKind::Quit },
        ],
        MenuId::NewGame => &[
            MenuItem { label: "I'm too young to die.", kind: MenuItemKind::Skill(0) },
            MenuItem { label: "Hey, not too rough.", kind: MenuItemKind::Skill(1) },
            MenuItem { label: "Hurt me plenty.", kind: MenuItemKind::Skill(2) },
            MenuItem { label: "Ultra-Violence.", kind: MenuItemKind::Skill(3) },
            MenuItem { label: "NIGHTMARE!", kind: MenuItemKind::Skill(4) },
        ],
        MenuId::Episode => &[
            MenuItem { label: "Knee-Deep in the Dead", kind: MenuItemKind::Episode(1) },
        ],
        MenuId::Options | MenuId::Sound | MenuId::Quit | MenuId::None => &[],
    }
}

/// What the menu asked the engine to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// `M_StartGame`: skill chosen; episode decided already (shareware: 1).
    StartGame { skill: i32, episode: i32 },
    /// Sound volume changes (`M_SfxVol`/`M_MusicVol` steps).
    ChangeSfxVolume(i32),
    ChangeMusicVolume(i32),
    /// The player confirmed quit (`M_QuitDOOM`).
    Quit,
}

/// Menu input events (`MenuInput` maps raw keys to these; the host decides
/// the mapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuInput {
    Up,
    Down,
    Left,
    Right,
    /// Enter/`BT_USE` on the highlighted item.
    Select,
    /// Escape/back.
    Back,
}

/// `menuframework_t` + the menu stack state.
#[derive(Clone, Debug)]
pub struct MenuState {
    pub active: bool,
    pub current: MenuId,
    /// Item shortcut (1..9 pressed on a menu screen).
    pub cursor: usize,
    /// `saveSlot`-style pending choice state lives in the host.
    quit_confirm: bool,
}

impl Default for MenuState {
    fn default() -> Self {
        MenuState {
            active: false,
            current: MenuId::None,
            cursor: 0,
            quit_confirm: false,
        }
    }
}

impl MenuState {
    pub fn new() -> Self {
        Self::default()
    }

    /// `M_StartControlPanel`.
    pub fn open(&mut self) {
        self.active = true;
        self.current = MenuId::Main;
        self.cursor = 0;
        self.quit_confirm = false;
    }

    /// `M_ClearMenus`.
    pub fn close(&mut self) {
        self.active = false;
        self.current = MenuId::None;
        self.quit_confirm = false;
    }

    fn menu(&self) -> &'static [MenuItem] {
        items(self.current)
    }

    /// `M_Ticker` + `M_Responder` fused: apply one input event, returning
    /// the actions the engine should perform.
    pub fn input(&mut self, ev: MenuInput) -> Vec<MenuAction> {
        if !self.active {
            if ev == MenuInput::Select || ev == MenuInput::Back {
                return Vec::new();
            }
            self.open();
        }

        let menu = self.menu();
        match ev {
            MenuInput::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            MenuInput::Down => {
                if self.cursor + 1 < menu.len() {
                    self.cursor += 1;
                }
            }
            MenuInput::Back => {
                if self.current == MenuId::Main {
                    self.close();
                } else {
                    self.current = MenuId::Main;
                    self.cursor = 0;
                }
            }
            MenuInput::Left | MenuInput::Right => {
                // Volume adjust on the (host-drawn) options screen; the core
                // only exports the deltas.
                if self.current == MenuId::Sound {
                    let d = if ev == MenuInput::Right { 1 } else { -1 };
                    return alloc::vec![
                        MenuAction::ChangeSfxVolume(d * (DEFAULT_SFX_VOLUME / 4)),
                        MenuAction::ChangeMusicVolume(d * (DEFAULT_MUSIC_VOLUME / 4)),
                    ];
                }
            }
            MenuInput::Select => {
                let Some(item) = menu.get(self.cursor) else {
                    return Vec::new();
                };
                match item.kind {
                    MenuItemKind::Submenu(id) => {
                        self.current = id;
                        self.cursor = 0;
                    }
                    MenuItemKind::Skill(skill) => {
                        self.close();
                        // Shareware IWADs only have episode 1 (the host can
                        // re-emit with a chosen episode).
                        return alloc::vec![MenuAction::StartGame { skill, episode: 1 }];
                    }
                    MenuItemKind::Episode(ep) => {
                        self.current = MenuId::NewGame;
                        self.cursor = 0;
                        let _ = ep;
                    }
                    MenuItemKind::Quit => {
                        if self.quit_confirm {
                            self.close();
                            return alloc::vec![MenuAction::Quit];
                        }
                        self.quit_confirm = true;
                    }
                    MenuItemKind::Separator => {}
                }
            }
        }
        Vec::new()
    }

    /// The highlighted item (for the host's drawer).
    pub fn selected(&self) -> Option<&'static MenuItem> {
        self.menu().get(self.cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_and_start_a_game() {
        let mut m = MenuState::new();
        m.open();
        assert_eq!(m.current, MenuId::Main);

        let acts = m.input(MenuInput::Select);
        assert_eq!(m.current, MenuId::NewGame);
        assert!(acts.is_empty());

        // Hurt me plenty.
        for _ in 0..2 {
            m.input(MenuInput::Down);
        }
        let acts = m.input(MenuInput::Select);
        assert_eq!(
            acts,
            alloc::vec![MenuAction::StartGame { skill: 2, episode: 1 }]
        );
        assert!(!m.active);
    }

    #[test]
    fn quit_needs_confirmation() {
        let mut m = MenuState::new();
        m.open();
        m.input(MenuInput::Down);
        m.input(MenuInput::Down);
        assert!(m.input(MenuInput::Select).is_empty());
        assert_eq!(m.input(MenuInput::Select), alloc::vec![MenuAction::Quit]);
    }

    #[test]
    fn back_leaves_submenus() {
        let mut m = MenuState::new();
        m.open();
        m.input(MenuInput::Select); // New Game submenu
        m.input(MenuInput::Back);
        assert_eq!(m.current, MenuId::Main);
        m.input(MenuInput::Back);
        assert!(!m.active);
    }
}
