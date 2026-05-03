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
    bg: 0xf7f7f8,
    surface: 0xffffff,
    sidebar: 0xf1f1f3,
    sidebar_active: 0xe4e4e7,
    fg: 0x222225,
    muted: 0x8a8a91,
    accent: 0xff6a1a,
    border: 0xdadade,
};
