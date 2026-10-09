// Colour themes for the TUI: the built-in ones, and the user's own (themes/<name>.toml in the config directory, each
// over another with `inherits`, every token and the roles of modisa's chrome its to set).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use serde::Serialize;
use serde_json::Value;

use super::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Theme {
    pub bg: &'static str,
    pub bar: &'static str,
    pub fg: &'static str,
    pub dim: &'static str,
    pub border: &'static str,
    pub focus: &'static str,
    pub accent: &'static str,
    pub warn: &'static str,
    pub blocked: &'static str,
    pub working: &'static str,
    pub done: &'static str,
    pub idle: &'static str,
}

// In the order the settings page lists them.
pub const THEMES: &[(&str, Theme)] = &[
    ("ion", Theme { bg: "#090f1b", bar: "#101b2c", fg: "#d9e7f5", dim: "#8295ad", border: "#293c54", focus: "#5ee7ef", accent: "#b39aff", warn: "#f0c674", blocked: "#ff7f96", working: "#5ee7ef", done: "#a5efb5", idle: "#8295ad" }),
    ("tokyonight", Theme { bg: "#1a1b26", bar: "#16161e", fg: "#c0caf5", dim: "#565f89", border: "#3b4261", focus: "#7aa2f7", accent: "#7aa2f7", warn: "#e0af68", blocked: "#f7768e", working: "#e0af68", done: "#7dcfff", idle: "#9ece6a" }),
    ("catppuccin-mocha", Theme { bg: "#1e1e2e", bar: "#181825", fg: "#cdd6f4", dim: "#6c7086", border: "#45475a", focus: "#cba6f7", accent: "#cba6f7", warn: "#f9e2af", blocked: "#f38ba8", working: "#f9e2af", done: "#89b4fa", idle: "#a6e3a1" }),
    ("gruvbox", Theme { bg: "#282828", bar: "#1d2021", fg: "#ebdbb2", dim: "#928374", border: "#504945", focus: "#fabd2f", accent: "#fabd2f", warn: "#fe8019", blocked: "#fb4934", working: "#fabd2f", done: "#83a598", idle: "#b8bb26" }),
    ("nord", Theme { bg: "#2e3440", bar: "#242933", fg: "#eceff4", dim: "#616e88", border: "#434c5e", focus: "#88c0d0", accent: "#88c0d0", warn: "#ebcb8b", blocked: "#bf616a", working: "#ebcb8b", done: "#81a1c1", idle: "#a3be8c" }),
    ("dracula", Theme { bg: "#282a36", bar: "#21222c", fg: "#f8f8f2", dim: "#6272a4", border: "#44475a", focus: "#bd93f9", accent: "#bd93f9", warn: "#f1fa8c", blocked: "#ff5555", working: "#f1fa8c", done: "#8be9fd", idle: "#50fa7b" }),
    ("catppuccin-latte", Theme { bg: "#eff1f5", bar: "#e6e9ef", fg: "#4c4f69", dim: "#8c8fa1", border: "#bcc0cc", focus: "#8839ef", accent: "#8839ef", warn: "#df8e1d", blocked: "#d20f39", working: "#df8e1d", done: "#1e66f5", idle: "#40a02b" }),
    ("github-light", Theme { bg: "#ffffff", bar: "#f6f8fa", fg: "#24292f", dim: "#6e7781", border: "#d0d7de", focus: "#0969da", accent: "#8250df", warn: "#9a6700", blocked: "#cf222e", working: "#bf8700", done: "#0969da", idle: "#1a7f37" }),
    ("tokyonight-day", Theme { bg: "#e1e2e7", bar: "#d0d5e3", fg: "#3760bf", dim: "#848cb5", border: "#a8aecb", focus: "#2e7de9", accent: "#9854f1", warn: "#8c6c3e", blocked: "#f52a65", working: "#8c6c3e", done: "#007197", idle: "#587539" }),
    ("solarized-light", Theme { bg: "#fdf6e3", bar: "#eee8d5", fg: "#586e75", dim: "#93a1a1", border: "#d3cbb7", focus: "#268bd2", accent: "#6c71c4", warn: "#b58900", blocked: "#dc322f", working: "#b58900", done: "#268bd2", idle: "#859900" }),
    ("gruvbox-light", Theme { bg: "#fbf1c7", bar: "#f2e5bc", fg: "#3c3836", dim: "#928374", border: "#d5c4a1", focus: "#b57614", accent: "#8f3f71", warn: "#af3a03", blocked: "#9d0006", working: "#b57614", done: "#076678", idle: "#79740e" }),
    // Bearded Theme (github.com/BeardedBear/bearded-theme, MIT): its UI and level colours mapped onto these roles.
    ("bearded-arc", Theme { bg: "#1c2433", bar: "#181f2c", fg: "#d0d7e4", dim: "#707786", border: "#3c4353", focus: "#8196b5", accent: "#b78aff", warn: "#ff955c", blocked: "#e35535", working: "#69c3ff", done: "#3cec85", idle: "#707786" }),
    ("bearded-arc-eolstorm", Theme { bg: "#222a38", bar: "#1e2531", fg: "#d8dde7", dim: "#777d8a", border: "#424a57", focus: "#9dacc3", accent: "#b78aff", warn: "#ff955c", blocked: "#e35535", working: "#69c3ff", done: "#3cec85", idle: "#777d8a" }),
    ("bearded-arc-blueberry", Theme { bg: "#111422", bar: "#0d101b", fg: "#bcc1dc", dim: "#606478", border: "#2e3242", focus: "#8eb0e6", accent: "#b78aff", warn: "#ff955c", blocked: "#e35535", working: "#69c3ff", done: "#3cec85", idle: "#606478" }),
    ("bearded-arc-eggplant", Theme { bg: "#181421", bar: "#13101a", fg: "#c8c1d9", dim: "#696376", border: "#363241", focus: "#9698d8", accent: "#b78aff", warn: "#ff955c", blocked: "#e35535", working: "#69c3ff", done: "#3cec85", idle: "#696376" }),
    ("bearded-arc-reversed", Theme { bg: "#121721", bar: "#161c28", fg: "#c5cdde", dim: "#646b78", border: "#313642", focus: "#8196b5", accent: "#b78aff", warn: "#ff955c", blocked: "#e35535", working: "#69c3ff", done: "#3cec85", idle: "#646b78" }),
    ("bearded-oceanic", Theme { bg: "#1a2b34", bar: "#16252d", fg: "#cddde6", dim: "#6e7e88", border: "#3a4a54", focus: "#97c892", accent: "#978dd6", warn: "#dc8255", blocked: "#ee5d75", working: "#5fb2df", done: "#97c892", idle: "#6e7e88" }),
    ("bearded-oceanic-reversed", Theme { bg: "#111c22", bar: "#152229", fg: "#c3d6e0", dim: "#63727a", border: "#303c43", focus: "#97c892", accent: "#978dd6", warn: "#dc8255", blocked: "#ee5d75", working: "#5fb2df", done: "#97c892", idle: "#63727a" }),
    ("bearded-solarized-dark", Theme { bg: "#132c34", bar: "#10252c", fg: "#c3e0e9", dim: "#668089", border: "#334c54", focus: "#47cfc4", accent: "#858bf7", warn: "#e2ae10", blocked: "#f45645", working: "#4db0f7", done: "#a5b82e", idle: "#668089" }),
    ("bearded-solarized-light", Theme { bg: "#fdf6e3", bar: "#f4edda", fg: "#000000", dim: "#76736b", border: "#c5bfb1", focus: "#2aa198", accent: "#666bd6", warn: "#e2ae10", blocked: "#f45645", working: "#4db0f7", done: "#a5b82e", idle: "#76736b" }),
    ("bearded-solarized-reversed", Theme { bg: "#0d1a20", bar: "#102128", fg: "#bad7e3", dim: "#5d717a", border: "#2b3a42", focus: "#47cfc4", accent: "#858bf7", warn: "#e2ae10", blocked: "#f45645", working: "#4db0f7", done: "#a5b82e", idle: "#5d717a" }),
    ("bearded-black-amethyst", Theme { bg: "#111418", bar: "#0c0f11", fg: "#bec6d0", dim: "#61666d", border: "#2f3337", focus: "#a85ff1", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#61666d" }),
    ("bearded-black-amethyst-soft", Theme { bg: "#171626", bar: "#13121f", fg: "#c6c5dc", dim: "#68667b", border: "#353446", focus: "#a85ff1", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#68667b" }),
    ("bearded-black-diamond", Theme { bg: "#111418", bar: "#0c0f11", fg: "#bec6d0", dim: "#61666d", border: "#2f3337", focus: "#11b7d4", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#61666d" }),
    ("bearded-black-diamond-soft", Theme { bg: "#161d26", bar: "#12181f", fg: "#c5cfdc", dim: "#676f7b", border: "#343c46", focus: "#11b7d4", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#676f7b" }),
    ("bearded-black-emerald", Theme { bg: "#111418", bar: "#0c0f11", fg: "#bec6d0", dim: "#61666d", border: "#2f3337", focus: "#38c7bd", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#61666d" }),
    ("bearded-black-emerald-soft", Theme { bg: "#162226", bar: "#121c1f", fg: "#c5d6dc", dim: "#67767b", border: "#344146", focus: "#38c7bd", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#67767b" }),
    ("bearded-black-gold", Theme { bg: "#111418", bar: "#0c0f11", fg: "#bec6d0", dim: "#61666d", border: "#2f3337", focus: "#c7910c", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#61666d" }),
    ("bearded-black-gold-soft", Theme { bg: "#221f1d", bar: "#1c1918", fg: "#d5d1cf", dim: "#75716f", border: "#413e3c", focus: "#c7910c", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#75716f" }),
    ("bearded-black-ruby", Theme { bg: "#111418", bar: "#0c0f11", fg: "#bec6d0", dim: "#61666d", border: "#2f3337", focus: "#c62f52", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#61666d" }),
    ("bearded-black-ruby-soft", Theme { bg: "#281a21", bar: "#21161b", fg: "#dccbd3", dim: "#7c6c74", border: "#483940", focus: "#c62f52", accent: "#a85ff1", warn: "#d4770c", blocked: "#e35535", working: "#11b7d4", done: "#00a884", idle: "#7c6c74" }),
    ("bearded-stained-purple", Theme { bg: "#20192b", bar: "#1b1524", fg: "#d2cadf", dim: "#736b7f", border: "#3f384b", focus: "#a948ef", accent: "#935cd1", warn: "#c9a022", blocked: "#c13838", working: "#3398db", done: "#37ae6f", idle: "#736b7f" }),
    ("bearded-stained-blue", Theme { bg: "#121726", bar: "#0e121e", fg: "#bec6df", dim: "#61677c", border: "#303546", focus: "#3a7fff", accent: "#935cd1", warn: "#c9a022", blocked: "#c13838", working: "#3398db", done: "#37ae6f", idle: "#61677c" }),
    ("bearded-vivid-purple", Theme { bg: "#171131", bar: "#130e29", fg: "#c7bfe8", dim: "#696187", border: "#362f52", focus: "#a680ff", accent: "#a95eff", warn: "#ffb638", blocked: "#d62c2c", working: "#28a9ff", done: "#42dd76", idle: "#696187" }),
    ("bearded-vivid-black", Theme { bg: "#141417", bar: "#0f0f11", fg: "#c5c5cb", dim: "#65656a", border: "#323236", focus: "#aaaaaa", accent: "#a95eff", warn: "#ffb638", blocked: "#d62c2c", working: "#28a9ff", done: "#42dd76", idle: "#65656a" }),
    ("bearded-vivid-light", Theme { bg: "#f4f4f4", bar: "#ebebeb", fg: "#181818", dim: "#7d7d7d", border: "#c2c2c2", focus: "#7e7e7e", accent: "#9c45ff", warn: "#ffb638", blocked: "#d62c2c", working: "#28a9ff", done: "#42dd76", idle: "#7d7d7d" }),
    ("bearded-monokai-terra", Theme { bg: "#262329", bar: "#201e23", fg: "#d9d6db", dim: "#79767c", border: "#454248", focus: "#b0a2a6", accent: "#ab9df2", warn: "#ffd866", blocked: "#fc6a67", working: "#78dce8", done: "#a9dc76", idle: "#79767c" }),
    ("bearded-monokai-metallian", Theme { bg: "#1e212b", bar: "#191c24", fg: "#d0d3de", dim: "#71747f", border: "#3d404b", focus: "#98a2b5", accent: "#ab9df2", warn: "#ffd866", blocked: "#fc6a67", working: "#78dce8", done: "#a9dc76", idle: "#71747f" }),
    ("bearded-monokai-stone", Theme { bg: "#2a2d33", bar: "#25282d", fg: "#dee0e4", dim: "#7e8186", border: "#4a4d53", focus: "#9aa2a6", accent: "#ab9df2", warn: "#ffd866", blocked: "#fc6a67", working: "#78dce8", done: "#a9dc76", idle: "#7e8186" }),
    ("bearded-monokai-black", Theme { bg: "#141414", bar: "#0e0e0e", fg: "#c7c7c7", dim: "#666666", border: "#333333", focus: "#8f8f8f", accent: "#ab9df2", warn: "#ffd866", blocked: "#fc6a67", working: "#78dce8", done: "#a9dc76", idle: "#666666" }),
    ("bearded-monokai-reversed", Theme { bg: "#12141a", bar: "#171921", fg: "#c6cad7", dim: "#656871", border: "#31333a", focus: "#98a2b5", accent: "#ab9df2", warn: "#ffd866", blocked: "#fc6a67", working: "#78dce8", done: "#a9dc76", idle: "#656871" }),
    ("bearded-earth", Theme { bg: "#221b1b", bar: "#1c1616", fg: "#caa5a5", dim: "#705b5b", border: "#3f3333", focus: "#d35386", accent: "#bab13b", warn: "#cc8c39", blocked: "#c13838", working: "#04c4d9", done: "#14b871", idle: "#705b5b" }),
    ("bearded-coffee", Theme { bg: "#292423", bar: "#231f1e", fg: "#ceb5b0", dim: "#766865", border: "#463e3c", focus: "#f09177", accent: "#9991f1", warn: "#ffa777", blocked: "#f24343", working: "#6eddd6", done: "#94d652", idle: "#766865" }),
    ("bearded-coffee-reversed", Theme { bg: "#1a1716", bar: "#201c1b", fg: "#c8aca5", dim: "#6a5c58", border: "#38312f", focus: "#f09177", accent: "#9991f1", warn: "#ffa777", blocked: "#f24343", working: "#6eddd6", done: "#94d652", idle: "#6a5c58" }),
    ("bearded-coffee-cream", Theme { bg: "#eae4e1", bar: "#e3dbd7", fg: "#36221d", dim: "#8c7c77", border: "#c3b9b5", focus: "#d3694c", accent: "#7056c4", warn: "#ce6700", blocked: "#ff3a3a", working: "#009db5", done: "#51a200", idle: "#8c7c77" }),
    ("bearded-void", Theme { bg: "#171322", bar: "#120f1b", fg: "#c7bfdb", dim: "#686278", border: "#353142", focus: "#7a63ed", accent: "#2bd3e2", warn: "#cc8c39", blocked: "#c13838", working: "#04c4d9", done: "#14b871", idle: "#686278" }),
    ("bearded-altica", Theme { bg: "#0f1c21", bar: "#0e171c", fg: "#c2ced1", dim: "#626e73", border: "#2e3b40", focus: "#0187a6", accent: "#9c8acf", warn: "#cc8c39", blocked: "#c13838", working: "#04c4d9", done: "#14b871", idle: "#626e73" }),
    ("bearded-feat-will", Theme { bg: "#14111f", bar: "#0d0a14", fg: "#bdb6d3", dim: "#625d72", border: "#312e3e", focus: "#b498f5", accent: "#c39eff", warn: "#ffae82", blocked: "#f7775a", working: "#8ad0ff", done: "#5fee9b", idle: "#625d72" }),
    ("bearded-feat-gold-d-raynh", Theme { bg: "#0f1628", bar: "#0c1220", fg: "#b8c4e4", dim: "#5d6680", border: "#2c3449", focus: "#e39000", accent: "#a167ff", warn: "#ff823f", blocked: "#f7775a", working: "#3eb2ff", done: "#21ff7d", idle: "#5d6680" }),
    ("bearded-feat-gold-d-raynh-light", Theme { bg: "#f5f5f5", bar: "#ececec", fg: "#0f212d", dim: "#7a838a", border: "#c2c6c9", focus: "#2397e5", accent: "#7537d7", warn: "#c0571f", blocked: "#f7775a", working: "#037ed1", done: "#03810d", idle: "#7a838a" }),
    ("bearded-feat-mellejulie", Theme { bg: "#1c1f24", bar: "#171a1e", fg: "#cdd1d8", dim: "#6e7178", border: "#3b3e43", focus: "#63edef", accent: "#968ffb", warn: "#edb492", blocked: "#e55454", working: "#63c0ff", done: "#71e893", idle: "#6e7178" }),
    ("bearded-feat-mellejulie-light", Theme { bg: "#edeeee", bar: "#e4e5e5", fg: "#000000", dim: "#6f7070", border: "#b9b9b9", focus: "#218d8f", accent: "#7c68ef", warn: "#c97a2a", blocked: "#d24545", working: "#1f89cf", done: "#2aa54d", idle: "#6f7070" }),
    ("bearded-feat-webevody", Theme { bg: "#00171a", bar: "#000d0f", fg: "#81f0fe", dim: "#3d7a82", border: "#183b41", focus: "#e95d74", accent: "#f75f94", warn: "#e3946a", blocked: "#e61e3f", working: "#f1d868", done: "#60e66f", idle: "#3d7a82" }),
    ("bearded-classics-anthracite", Theme { bg: "#181a1f", bar: "#131519", fg: "#c8ccd4", dim: "#696c73", border: "#36393e", focus: "#a2abb6", accent: "#935cd1", warn: "#c9a022", blocked: "#c13838", working: "#3398db", done: "#37ae6f", idle: "#696c73" }),
    ("bearded-classics-light", Theme { bg: "#f3f4f5", bar: "#e9ebed", fg: "#091316", dim: "#757a7c", border: "#bfc1c3", focus: "#22a5c9", accent: "#8737e6", warn: "#bc7400", blocked: "#ac2121", working: "#0468bf", done: "#14852a", idle: "#757a7c" }),
    ("bearded-surprising-eggplant", Theme { bg: "#1d1426", bar: "#17101f", fg: "#d0c1de", dim: "#70647c", border: "#3c3246", focus: "#d24e4e", accent: "#cc9b52", warn: "#d1a456", blocked: "#e35535", working: "#00b3bd", done: "#a9dc76", idle: "#70647c" }),
    ("bearded-surprising-blueberry", Theme { bg: "#101a29", bar: "#0d1521", fg: "#bacbe4", dim: "#5f6c80", border: "#2e384a", focus: "#c93e71", accent: "#cc9b52", warn: "#d1a456", blocked: "#e35535", working: "#00b3bd", done: "#a9dc76", idle: "#5f6c80" }),
    ("bearded-surprising-watermelon", Theme { bg: "#142326", bar: "#101c1f", fg: "#c1d9de", dim: "#64777c", border: "#324346", focus: "#da6c62", accent: "#cc9b52", warn: "#d1a456", blocked: "#e35535", working: "#00b3bd", done: "#a9dc76", idle: "#64777c" }),
    ("bearded-hc-ebony", Theme { bg: "#181820", bar: "#13131a", fg: "#c8c8d5", dim: "#696974", border: "#36363f", focus: "#dbdeea", accent: "#bc98ff", warn: "#ffaa7d", blocked: "#fd604f", working: "#7fd7f5", done: "#afea7b", idle: "#696974" }),
    ("bearded-hc-midnightvoid", Theme { bg: "#151f27", bar: "#111920", fg: "#c3d2de", dim: "#65727c", border: "#333e47", focus: "#dbefff", accent: "#bc98ff", warn: "#ffaa7d", blocked: "#fd604f", working: "#7fd7f5", done: "#afea7b", idle: "#65727c" }),
    ("bearded-hc-flurry", Theme { bg: "#f5f8fc", bar: "#eaecee", fg: "#272d34", dim: "#878b91", border: "#c7cbd0", focus: "#444c54", accent: "#b377e3", warn: "#e3946a", blocked: "#ee5f50", working: "#0aa3d6", done: "#41ad4e", idle: "#878b91" }),
    ("bearded-hc-wonderland-wood", Theme { bg: "#1f1d36", bar: "#1b192f", fg: "#d3d2e6", dim: "#737189", border: "#3f3c56", focus: "#fbe7c3", accent: "#9a94e9", warn: "#e4a792", blocked: "#ff7e70", working: "#92b4ff", done: "#91d6a7", idle: "#737189" }),
    ("bearded-hc-brewing-storm", Theme { bg: "#0c2a42", bar: "#0a2439", fg: "#c0dcf3", dim: "#637d96", border: "#2f4962", focus: "#9dffd9", accent: "#b8b3ff", warn: "#ff9d7c", blocked: "#ff5e4c", working: "#3391e3", done: "#84ffad", idle: "#637d96" }),
    ("bearded-hc-minuit", Theme { bg: "#1c1827", bar: "#171420", fg: "#cdc8dc", dim: "#6e697b", border: "#3b3647", focus: "#ecc48c", accent: "#ad92ff", warn: "#eea67f", blocked: "#fb7a6c", working: "#4fc1e8", done: "#74be7d", idle: "#6e697b" }),
    ("bearded-hc-chocolate-espresso", Theme { bg: "#2e2424", bar: "#281f1f", fg: "#d3b1b1", dim: "#7b6666", border: "#4b3d3d", focus: "#f69c95", accent: "#ad92ff", warn: "#eea67f", blocked: "#fb7a6c", working: "#4fc1e8", done: "#74be7d", idle: "#7b6666" }),
    ("bearded-milkshake-raspberry", Theme { bg: "#f1e8eb", bar: "#eadde1", fg: "#15070b", dim: "#796f72", border: "#bfb6b9", focus: "#d1174f", accent: "#7522d3", warn: "#c08403", blocked: "#d12525", working: "#0076c5", done: "#008b17", idle: "#796f72" }),
    ("bearded-milkshake-blueberry", Theme { bg: "#dad9eb", bar: "#cfcde5", fg: "#07060c", dim: "#696872", border: "#abaab9", focus: "#422eb0", accent: "#7522d3", warn: "#c08403", blocked: "#d12525", working: "#0076c5", done: "#008b17", idle: "#696872" }),
    ("bearded-milkshake-mango", Theme { bg: "#f3eae3", bar: "#eee1d7", fg: "#100a08", dim: "#77716d", border: "#c0b8b2", focus: "#bd4f27", accent: "#7522d3", warn: "#c08403", blocked: "#d12525", working: "#0076c5", done: "#008b17", idle: "#77716d" }),
    ("bearded-milkshake-mint", Theme { bg: "#edf3ee", bar: "#e2ece4", fg: "#000000", dim: "#6f7270", border: "#b9bdb9", focus: "#2a9b7d", accent: "#7522d3", warn: "#c08403", blocked: "#d12525", working: "#0076c5", done: "#008b17", idle: "#6f7270" }),
    ("bearded-milkshake-vanilla", Theme { bg: "#ece7da", bar: "#e6dfce", fg: "#000000", dim: "#6f6c67", border: "#b8b4aa", focus: "#937416", accent: "#7522d3", warn: "#c08403", blocked: "#d12525", working: "#0076c5", done: "#008b17", idle: "#6f6c67" }),
    ("bearded-hemanopia", Theme { bg: "#1b1e28", bar: "#161921", fg: "#ccd0dc", dim: "#6d707c", border: "#3a3d48", focus: "#9887eb", accent: "#5f77dc", warn: "#ffaf86", blocked: "#ff5b82", working: "#32a7ff", done: "#38ffa1", idle: "#6d707c" }),
    ("bearded-aquarelle-cymbidium", Theme { bg: "#2c252a", bar: "#262024", fg: "#ded8dc", dim: "#7f787d", border: "#4c4449", focus: "#da6e6c", accent: "#bcb1f1", warn: "#eea064", blocked: "#e87a70", working: "#73bee9", done: "#aada77", idle: "#7f787d" }),
    ("bearded-aquarelle-hydrangea", Theme { bg: "#22273c", bar: "#1e2235", fg: "#dadde9", dim: "#787c8e", border: "#43475b", focus: "#6394f1", accent: "#bcb1f1", warn: "#eea064", blocked: "#e87a70", working: "#73bee9", done: "#aada77", idle: "#787c8e" }),
    ("bearded-aquarelle-lilac", Theme { bg: "#252433", bar: "#201f2c", fg: "#d9d9e3", dim: "#797886", border: "#454452", focus: "#9587ff", accent: "#bcb1f1", warn: "#eea064", blocked: "#e87a70", working: "#73bee9", done: "#aada77", idle: "#797886" }),
    ("bearded-oled", Theme { bg: "#000000", bar: "#000000", fg: "#b3b3b3", dim: "#565656", border: "#252525", focus: "#688eff", accent: "#b69ede", warn: "#e79e69", blocked: "#e87474", working: "#63bbe5", done: "#5cd4c3", idle: "#565656" }),
];

pub fn find_theme(name: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|(n, _)| *n == name).map(|(_, t)| t).or_else(|| CUSTOM.read().ok()?.iter().find(|c| c.name == name).map(|c| c.theme))
}

// The terminal's own background is light: theme = { dark, light } then takes the light one. The client says so when it
// starts (it asks the terminal, before it paints its own background).
pub static LIGHT: AtomicBool = AtomicBool::new(false);

pub fn theme(c: &Config) -> &'static Theme {
    find_theme(active_name(c)).or_else(|| find_theme("tokyonight")).expect("tokyonight is built in")
}

// The name of the theme in use: the light one of a pair on a light terminal.
pub fn active_name(c: &Config) -> &str {
    match &c.theme_light {
        Some(light) if LIGHT.load(Ordering::Relaxed) => light,
        _ => &c.theme,
    }
}

// Every theme there is, built-in first, by name.
pub fn all() -> Vec<(String, &'static Theme)> {
    let mut out: Vec<(String, &'static Theme)> = THEMES.iter().map(|(n, t)| (n.to_string(), t)).collect();
    out.extend(CUSTOM.read().map(|c| c.iter().map(|c| (c.name.to_string(), c.theme)).collect::<Vec<_>>()).unwrap_or_default());
    out
}

// ---------- the user's own themes, and roles ----------

// How one part of modisa's chrome looks, over what the tokens give it: a theme file's [roles].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Role {
    pub fg: Option<&'static str>,
    pub bg: Option<&'static str>,
    pub bold: bool,
}

pub const ROLES: &[&str] = &["tab.active", "tab.inactive", "pane.border", "pane.border.focused", "pane.title", "sidebar", "sidebar.selected", "status", "menu", "menu.selected", "toast"];
const TOKENS: [&str; 12] = ["bg", "bar", "fg", "dim", "border", "focus", "accent", "warn", "blocked", "working", "done", "idle"];

struct Custom {
    name: &'static str,
    theme: &'static Theme,
    roles: Vec<(&'static str, Role)>,
}

// ponytail: a theme loaded is kept for good (its strings must live as long as the built-in ones); each reload of a
// changed file keeps a few hundred bytes more
static CUSTOM: RwLock<Vec<Custom>> = RwLock::new(Vec::new());

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub fn themes_dir() -> String {
    format!("{}/themes", *super::CONFIG_DIR)
}

fn hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn token_of(t: &Theme, name: &str) -> Option<&'static str> {
    Some(match name {
        "bg" => t.bg,
        "bar" => t.bar,
        "fg" => t.fg,
        "dim" => t.dim,
        "border" => t.border,
        "focus" => t.focus,
        "accent" => t.accent,
        "warn" => t.warn,
        "blocked" => t.blocked,
        "working" => t.working,
        "done" => t.done,
        "idle" => t.idle,
        _ => return None,
    })
}

// "#rrggbb", or "$token" of the theme it's over
fn colour(v: &str, base: &Theme) -> Result<&'static str, String> {
    if hex(v) {
        return Ok(leak(v.to_lowercase()));
    }
    v.strip_prefix('$').and_then(|t| token_of(base, t)).ok_or_else(|| format!("{v:?} isn't #rrggbb or a $token ({})", TOKENS.join(", ")))
}

// "bold $accent on #1a1b26"
fn role(v: &str, t: &Theme) -> Result<Role, String> {
    let mut r = Role::default();
    let mut words = v.split_whitespace();
    while let Some(w) = words.next() {
        match w {
            "bold" => r.bold = true,
            "on" => r.bg = Some(colour(words.next().ok_or("\"on\" needs a colour after it")?, t)?),
            c if r.fg.is_none() => r.fg = Some(colour(c, t)?),
            c => return Err(format!("{c:?}: one foreground colour, then \"on\" and the background")),
        }
    }
    Ok(r)
}

// One theme file's settings over `base`: its tokens, then its roles (whose $tokens are its own).
fn build(file: &Value, base: &Theme) -> Result<(Theme, Vec<(&'static str, Role)>), String> {
    let mut t = *base;
    let table = file.as_object().ok_or("a theme file is a table")?;
    for (k, v) in table {
        if k == "inherits" || k == "roles" {
            continue;
        }
        let Some(slot) = (match k.as_str() {
            "bg" => Some(&mut t.bg),
            "bar" => Some(&mut t.bar),
            "fg" => Some(&mut t.fg),
            "dim" => Some(&mut t.dim),
            "border" => Some(&mut t.border),
            "focus" => Some(&mut t.focus),
            "accent" => Some(&mut t.accent),
            "warn" => Some(&mut t.warn),
            "blocked" => Some(&mut t.blocked),
            "working" => Some(&mut t.working),
            "done" => Some(&mut t.done),
            "idle" => Some(&mut t.idle),
            _ => None,
        }) else {
            return Err(format!("{k} isn't a theme's token ({}), inherits or roles", TOKENS.join(", ")));
        };
        *slot = colour(v.as_str().ok_or_else(|| format!("{k} is a colour, \"#rrggbb\""))?, base).map_err(|e| format!("{k}: {e}"))?;
    }
    let mut roles = vec![];
    for (k, v) in file.get("roles").and_then(Value::as_object).into_iter().flatten() {
        let name = ROLES.iter().find(|r| **r == k.as_str()).ok_or_else(|| format!("roles.{k} isn't one of {}", ROLES.join(", ")))?;
        roles.push((*name, role(v.as_str().ok_or_else(|| format!("roles.{k} is a style, like \"bold $fg on $bar\""))?, &t).map_err(|e| format!("roles.{k}: {e}"))?));
    }
    Ok((t, roles))
}

// The themes directory read again (a theme over another of the user's is read after it): the problems, by file.
pub fn load_custom() -> Vec<(String, String)> {
    let mut files: Vec<(String, Value)> = vec![];
    let mut problems = vec![];
    let mut paths: Vec<_> = std::fs::read_dir(themes_dir()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect();
    paths.sort();
    for path in paths {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| super::parse_toml(&t).map_err(|e| e.message)) {
            Ok(_) if THEMES.iter().any(|(n, _)| *n == name) => problems.push((name, format!("a built-in theme is called {}: rename the file", path.display()))),
            Ok(m) => files.push((name, Value::Object(m))),
            Err(e) => problems.push((name, e)),
        }
    }
    let mut loaded: Vec<Custom> = vec![];
    // a file over another of the user's waits for it; what's left when nothing more loads is a loop or a missing one
    while !files.is_empty() {
        let before = files.len();
        files.retain(|(name, file)| {
            let over = file.get("inherits").and_then(Value::as_str).unwrap_or("tokyonight");
            let base = THEMES.iter().find(|(n, _)| *n == over).map(|(_, t)| t).or_else(|| loaded.iter().find(|c| c.name == over).map(|c| c.theme));
            let Some(base) = base else { return true };
            match build(file, base) {
                Ok((t, roles)) => loaded.push(Custom { name: leak(name.clone()), theme: Box::leak(Box::new(t)), roles }),
                Err(e) => problems.push((name.clone(), e)),
            }
            false
        });
        if files.len() == before {
            for (name, file) in files.drain(..) {
                problems.push((name, format!("it inherits {}, which isn't a theme (or inherits it back)", file.get("inherits").and_then(Value::as_str).unwrap_or("?"))));
            }
        }
    }
    if let Ok(mut c) = CUSTOM.write() {
        *c = loaded;
    }
    problems
}

// A theme's roles (none for a built-in one).
pub fn roles(name: &str) -> Vec<(&'static str, Role)> {
    CUSTOM.read().ok().and_then(|c| c.iter().find(|c| c.name == name).map(|c| c.roles.clone())).unwrap_or_default()
}
