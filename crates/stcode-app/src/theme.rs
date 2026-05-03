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
    bg: 0xfbfbfc,
    surface: 0xffffff,
    sidebar: 0xf6f6f7,
    sidebar_active: 0xe9e9ec,
    fg: 0x1d1d20,
    muted: 0x686870,
    accent: 0xff6a1a,
    border: 0xc9c9cf,
};
