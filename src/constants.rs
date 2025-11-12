pub const FETCHED_FNAME: &str = "raw.csv";
pub const HARMONICS: [usize; 6] = [1, 2, 3, 4, 5, 6];
pub const T_HEADER: &str = "time (s)";
pub const LI_HEADER: [&str; 12] = [
    "LIx_h1 (V)",
    "LIy_h1 (V)",
    "LIx_h2 (V)",
    "LIy_h2 (V)",
    "LIx_h3 (V)",
    "LIy_h3 (V)",
    "LIx_h4 (V)",
    "LIy_h4 (V)",
    "LIx_h5 (V)",
    "LIy_h5 (V)",
    "LIx_h6 (V)",
    "LIy_h6 (V)",
];

pub const LI_RESULTS_NAME: &str = "lockin_results";

pub const LI_ROTATED_HEADER: [&str; 12] = [
    "LIin_h1 (V)",
    "LIout_h1 (V)",
    "LIin_h2 (V)",
    "LIout_h2 (V)",
    "LIin_h3 (V)",
    "LIout_h3 (V)",
    "LIin_h4 (V)",
    "LIout_h4 (V)",
    "LIin_h5 (V)",
    "LIout_h5 (V)",
    "LIin_h6 (V)",
    "LIout_h6 (V)",
];
pub const LI_ROTATED_NAME: &str = "lockin_rotated";
