pub struct Tokens {
    pub bg: u32,
    pub surface: u32,
    /// 사이드바 — surface보다 살짝 어둡게.
    pub sidebar: u32,
    /// 사이드바의 활성/hover 항목 배경.
    pub sidebar_active: u32,
    pub fg: u32,
    pub muted: u32,
    pub accent: u32,
    /// 패널 경계선.
    pub border: u32,
}

pub const TOKENS: Tokens = Tokens {
    bg: 0xffffff,
    surface: 0xffffff,
    sidebar: 0xf8f8f9,
    sidebar_active: 0xebebef,
    fg: 0x17171a,
    muted: 0x6b6b73,
    accent: 0xf25f18,
    border: 0xe0e0e5,
};
