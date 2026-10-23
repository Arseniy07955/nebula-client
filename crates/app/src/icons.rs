//! Значки рисуются в коде: кружок-«планета» цвета состояния. Файлы картинок не нужны.

use common::ConnState;

pub const SIZE: u32 = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Up,
    Starting,
    Error,
    Down,
}

impl Look {
    pub fn of(state: Option<ConnState>) -> Look {
        match state {
            Some(ConnState::Running) => Look::Up,
            Some(ConnState::Starting) => Look::Starting,
            Some(ConnState::Error) => Look::Error,
            _ => Look::Down,
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Look::Up => [0x2e, 0xc2, 0x7e],
            Look::Starting => [0xe5, 0xa5, 0x0a],
            Look::Error => [0xe0, 0x4f, 0x4f],
            Look::Down => [0x8a, 0x8f, 0x98],
        }
    }
}

/// RGBA 32×32: диск с кольцом-орбитой, края сглажены.
pub fn rgba(look: Look) -> Vec<u8> {
    let [r, g, b] = look.rgb();
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    let c = (SIZE as f32 - 1.0) / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            // планета
            let disk = (9.5 - d).clamp(0.0, 1.0);
            // наклонная орбита: эллипс, повёрнутый на -25°
            let (s, co) = (-0.44f32, 0.9f32);
            let (u, v) = (dx * co - dy * s, dx * s + dy * co);
            let e = ((u / 15.0).powi(2) + (v / 5.5).powi(2)).sqrt();
            let ring = (1.0 - ((e - 1.0).abs() * 9.0)).clamp(0.0, 1.0) * if v < 0.0 && d < 10.5 { 0.0 } else { 1.0 };
            let a = disk.max(ring);
            if a > 0.0 {
                let i = ((y * SIZE + x) * 4) as usize;
                let light = if disk > 0.0 && dx + dy < -4.0 { 28 } else { 0 };
                px[i] = r.saturating_add(light);
                px[i + 1] = g.saturating_add(light);
                px[i + 2] = b.saturating_add(light);
                px[i + 3] = (a * 255.0) as u8;
            }
        }
    }
    px
}
