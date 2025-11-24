//! Shared constants used by the driver and instruments.

// --- NI-488.2 / Linux-GPIB Constants (Shared Logic) ---
pub const ERR: i32 = 0x8000;
pub const TIMO: i32 = 0x4000;
pub const END: i32 = 0x2000;

pub const NO_SAD: i32 = 0;
pub const EOT_ENABLE: i32 = 1;
pub const EOS_NONE: i32 = 0;

// --- VISA Constants (Windows) ---
#[cfg(target_os = "windows")]
pub mod visa {
    pub const VI_SUCCESS: i32 = 0;
    pub const VI_SUCCESS_TERM_CHAR: i32 = 0x3FFF0005;
    pub const VI_SUCCESS_MAX_CNT: i32 = 0x3FFF0006;
    
    pub const VI_NULL: u64 = 0;
    
    // Attributes
    pub const VI_ATTR_TMO_VALUE: u32 = 0x3FFF000A;
    pub const VI_ATTR_TERMCHAR: u32 = 0x3FFF0018;
    pub const VI_ATTR_TERMCHAR_EN: u32 = 0x3FFF0038;
    pub const VI_ATTR_SEND_END_EN: u32 = 0x3FFF0016;
    
    // Values
    pub const VI_TMO_INFINITE: u32 = 0xFFFFFFFF;
    
    // Error codes (subset)
    pub const VI_ERROR_TMO: i32 = -1073807339; // 0xBFFF0015
}