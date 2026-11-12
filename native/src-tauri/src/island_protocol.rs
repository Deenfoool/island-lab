// Versioned, pointer-free messages shared by Snowy New Year and Island Lab.
pub const MESSAGE: &str = "Deenfoool.SnowyNewYear.IslandControl.v1";
pub const CONTROLLER: &str = "SnowyNewYearController";
pub const ACK: usize = 0x534E_0000;

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Field {
    Falling = 1,
    Caps = 2,
    Snowballs = 3,
    Intensity = 4,
}

impl Field {
    pub fn decode(field: usize, value: isize) -> Option<(Self, u8)> {
        Some((
            match field {
                1 if (0..=1).contains(&value) => Self::Falling,
                2 if (0..=1).contains(&value) => Self::Caps,
                3 if (0..=1).contains(&value) => Self::Snowballs,
                4 if (0..=100).contains(&value) => Self::Intensity,
                _ => return None,
            },
            value as u8,
        ))
    }
}

pub fn pack(falling: bool, caps: bool, snowballs: bool, intensity: u8) -> usize {
    ACK | usize::from(falling)
        | (usize::from(caps) << 1)
        | (usize::from(snowballs) << 2)
        | (usize::from(intensity.min(100)) << 8)
}

// Four physical-pixel insets from the client rectangle to the visible top span.
pub const SNOW_EDGE_PROPERTY: &str = "Deenfoool.SnowyNewYear.VisibleTop.v1";
pub const SNOW_EDGE_HIDDEN: u32 = u32::MAX;
