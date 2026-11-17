//! Logical tool dimensions shared by each side, persisted with the normal session.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShellLayout {
    pub bottom_height: Option<f32>,
    pub left_width: Option<f32>,
    pub right_width: Option<f32>,
}

impl ShellLayout {
    pub(super) fn validate(self) -> Result<(), String> {
        for (value, min, max) in [
            (self.bottom_height, 160., 500.),
            (self.left_width, 150., 500.),
            (self.right_width, 220., 600.),
        ] {
            if value.is_some_and(|n| !n.is_finite() || !(min..=max).contains(&n)) {
                return Err("Dimensões de painéis inválidas".into());
            }
        }
        Ok(())
    }
}

impl AppService {
    pub fn shell_layout(&self) -> ShellLayout {
        self.shell_layout
    }
}
