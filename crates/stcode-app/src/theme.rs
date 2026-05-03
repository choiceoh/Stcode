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
    bg: 0x1e1e2e,
    surface: 0x282838,
    sidebar: 0x171722,
    sidebar_active: 0x2a2a40,
    fg: 0xeeeeee,
    muted: 0x9999aa,
    accent: 0x7aa2f7,
    border: 0x383848,
};
