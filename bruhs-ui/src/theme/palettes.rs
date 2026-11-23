use super::ThemeMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeFamily {
    Grove,
    Ocean,
    Ember,
    Iris,
    #[default]
    #[serde(other)]
    Bruhs,
}

/// Native surface, foreground and action ramps, ordered as the bruhs design tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub surface: [u32; 5],
    pub text: [u32; 6],
    pub accent: [u32; 6],
}

impl ThemeFamily {
    pub const ALL: [Self; 5] = [
        Self::Bruhs,
        Self::Grove,
        Self::Ocean,
        Self::Ember,
        Self::Iris,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bruhs => "Bruhs",
            Self::Grove => "Grove",
            Self::Ocean => "Ocean",
            Self::Ember => "Ember",
            Self::Iris => "Iris",
        }
    }
    pub const fn palette(self, mode: ThemeMode) -> &'static Palette {
        match (self, mode) {
            (Self::Bruhs, ThemeMode::Light) => &Palette {
                surface: [0xfaf6f1, 0xfffcf9, 0xe8dfd5, 0xdbcebf, 0xc7b6a5],
                text: [0x211b16, 0x302720, 0x44382e, 0x594a3e, 0x69594b, 0x806f60],
                accent: [0x5e2606, 0x773107, 0x893904, 0xa84209, 0xb94a0e, 0x83350b],
            },
            (Self::Bruhs, ThemeMode::Dark) => &Palette {
                surface: [0x141210, 0x1d1916, 0x2f2922, 0x443a2c, 0x594a36],
                text: [0xf5ede7, 0xede1d8, 0xdfcfc3, 0xc3afa1, 0xab998f, 0x8b7c74],
                accent: [0xfbe1cc, 0xf7c194, 0xf29e58, 0xec7f2c, 0xdf6815, 0xbd5310],
            },
            (Self::Grove, ThemeMode::Light) => &Palette {
                surface: [0xf3f7f4, 0xe2ede7, 0xecefed, 0xcbd5d1, 0xbecbc5],
                text: [0x241523, 0x2d1f2c, 0x3e323e, 0x59515a, 0x6c676d, 0x746c73],
                accent: [0x22302f, 0x20493a, 0x1d6244, 0x1a784d, 0x1a784d, 0x166742],
            },
            (Self::Grove, ThemeMode::Dark) => &Palette {
                surface: [0x1b2821, 0x21362b, 0x36413c, 0x415f4f, 0x4f725f],
                text: [0xfffaff, 0xf3f0f4, 0xdddcde, 0xb8bdbc, 0xa5aca9, 0x919595],
                accent: [0xd8f1e5, 0xb4e8cc, 0x90dfb4, 0x69d69a, 0x69d69a, 0x6eba90],
            },
            (Self::Ocean, ThemeMode::Light) => &Palette {
                surface: [0xf5f7f8, 0xe4ecf2, 0xedeff1, 0xcdd4dc, 0xc0c9d4],
                text: [0x241523, 0x2d1f2d, 0x3e323f, 0x5a515d, 0x6e6772, 0x746c75],
                accent: [0x252d47, 0x254469, 0x255a8b, 0x246da8, 0x246da8, 0x1f5d90],
            },
            (Self::Ocean, ThemeMode::Dark) => &Palette {
                surface: [0x17212b, 0x1e2d3b, 0x333b45, 0x405567, 0x4f677b],
                text: [0xfffaff, 0xf2eff4, 0xdad9e0, 0xb3b6bf, 0x9fa5ae, 0x8d8f97],
                accent: [0xdae9fb, 0xb8daf6, 0x95caf2, 0x70b9ee, 0x70b9ee, 0x78aacd],
            },
            (Self::Ember, ThemeMode::Light) => &Palette {
                surface: [0xf9f7f5, 0xf3eae5, 0xf1efee, 0xddd2ce, 0xd4c6c1],
                text: [0x241523, 0x2e1f2c, 0x40323e, 0x5e515a, 0x72666d, 0x766c74],
                accent: [0x482625, 0x693526, 0x8a4428, 0xa75228, 0xa75228, 0x8f4622],
            },
            (Self::Ember, ThemeMode::Dark) => &Palette {
                surface: [0x291e1a, 0x39281f, 0x433835, 0x664c3f, 0x7a5d4d],
                text: [0xfffaff, 0xf4eef2, 0xe0d8da, 0xbeb4b3, 0xada29f, 0x968e8f],
                accent: [0xfbe1d7, 0xf8cab2, 0xf4b38c, 0xf09a64, 0xf09a64, 0xd19875],
            },
            (Self::Iris, ThemeMode::Light) => &Palette {
                surface: [0xf8f7f9, 0xedeaf4, 0xf0eff2, 0xd6d1de, 0xccc5d6],
                text: [0x241523, 0x2d1f2d, 0x3f323f, 0x5c515d, 0x706672, 0x766c76],
                accent: [0x38254a, 0x4b346e, 0x5e4392, 0x7253b9, 0x7253b9, 0x5d4498],
            },
            (Self::Iris, ThemeMode::Dark) => &Palette {
                surface: [0x1d1929, 0x272139, 0x383443, 0x4d4366, 0x5d527b],
                text: [0xfffaff, 0xf2edf4, 0xdad5de, 0xb3aebb, 0xa39dac, 0x8e8a95],
                accent: [0xe6dafc, 0xcebcf8, 0xb69ef5, 0xab8ff4, 0xab8ff4, 0xa795d7],
            },
        }
    }
}
