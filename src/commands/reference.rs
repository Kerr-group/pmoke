use crate::config::Config;
use crate::lockin::reference::run;
use anyhow::Result;

pub fn reference(cfg: &Config) -> Result<()> {
    let _ = run(cfg)?;
    Ok(())
}

// pub fn reference(cfg: &Config) -> Result<()> {
//     let channels = build_channel_list(cfg)?;
//     let reference_ch = &cfg.roles.reference_ch;
//
//     if reference_ch.is_empty() {
//         bail!("Reference channel is not specified in the configuration.");
//     }
//     if reference_ch.len() != 1 {
//         bail!("Multiple reference channels are not supported.");
//     }
//
//     let col = channels
//         .iter()
//         .position(|ch| ch == &(reference_ch[0] as u8))
//         .ok_or_else(|| {
//             anyhow!(
//                 "Reference channel {} not found in the channel list.",
//                 reference_ch[0]
//             )
//         })?;
//
//     let start = Instant::now();
//     let data = read_selected_columns(FETCHED_FNAME, &[col])?;
//     let duration = start.elapsed();
//     let ref_data = &data[0];
//
//     let time = time_builder(cfg)?;
//
//     println!("Time elapsed in reading selected column: {:?}", duration);
//
//     // let channels = build_channel_list(cfg)?;
//     // let ncols = channels.len();
//     // let cols: &[usize] = &(0..ncols).collect::<Vec<usize>>();
//     //
//     // let start = Instant::now();
//     // let data = read_selected_columns(FETCHED_FNAME, cols)?;
//     // let duration = start.elapsed();
//     //
//     // println!("Time elapsed in reading selected columns: {:?}", duration);
//     // if !data.is_empty() {
//     //     let nrows = data[0].len();
//     //     if nrows > 0 {
//     //         let mut row_strings = Vec::with_capacity(ncols);
//     //         for col in 0..ncols {
//     //             let v = data[col][0]; // 0行目
//     //             row_strings.push(v.to_string());
//     //         }
//     //         println!("{}", row_strings.join(","));
//     //     }
//     // }
//
//     Ok(())
// }
