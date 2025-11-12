use crate::lockin::lockin_params::LockinParams;
use anyhow::Result;

#[derive(Clone, Copy)]
pub enum RefType {
    Sin,
    Cos,
}

#[allow(dead_code)]
pub struct LockinProcessor<'a> {
    t: &'a [f64],
    data: &'a [f64],
    omega_tref: f64,
    params: LockinParams,
}

impl<'a> LockinProcessor<'a> {
    pub fn new(
        t: &'a [f64],
        data: &'a [f64],
        f_ref: f64,
        omega_tref: f64,
        fil_length: usize,
        stride: usize,
    ) -> Result<Self> {
        assert!(t.len() >= 2);
        assert_eq!(t.len(), data.len());

        let params = LockinParams::from_slice(t, f_ref, fil_length, stride)?;

        Ok(Self {
            t,
            data,
            omega_tref,
            params,
        })
    }

    fn ref_signal(&self, t: f64, harmonic: usize, ref_type: RefType) -> f64 {
        let arg = (harmonic as f64) * (self.params.omega * t - self.omega_tref);
        match ref_type {
            RefType::Sin => arg.sin(),
            RefType::Cos => arg.cos(),
        }
    }

    pub fn compute_lockin(&self, harmonic: usize, ref_type: RefType) -> Vec<f64> {
        let mixed_signal: Vec<f64> = self
            .t
            .iter()
            .zip(self.data.iter())
            .map(|(&t, &data)| data * self.ref_signal(t, harmonic, ref_type))
            .collect();

        let i_start = 2 + (self.params.n_fil + 1) / self.params.stride;
        let i_end = self.params.n_int - i_start;
        let m = if i_end >= i_start {
            i_end - i_start + 1
        } else {
            0
        };
        let mut out = Vec::with_capacity(m);

        for k in 0..m {
            let i_idx = i_start + k;
            let i_base = i_idx * self.params.stride;

            let mut integ = 0.0;
            for j in 0..(2 * self.params.n_fil) {
                let j0 = j as isize - self.params.n_fil as isize;
                let j1 = j0 + 1;
                let idx0 = (i_base as isize + j0) as usize;
                let idx1 = (i_base as isize + j1) as usize;

                let f0 = mixed_signal[idx0];
                let f1 = mixed_signal[idx1];

                integ += 0.5 * (f0 + f1) * self.params.dt;
            }

            let neg_idx0 = i_base - self.params.n_fil;
            let neg_idx1 = i_base - self.params.n_fil - 1;

            let y0_neg = mixed_signal[neg_idx0];
            let y1_neg = mixed_signal[neg_idx1];

            let ym_neg = (y1_neg * self.params.diff_t
                + y0_neg * (self.params.dt - self.params.diff_t))
                / self.params.dt;
            let edge_neg = self.params.diff_t * 0.5 * (y0_neg + ym_neg);

            let pos_idx0 = i_base + self.params.n_fil;
            let pos_idx1 = i_base + self.params.n_fil + 1;

            let y0_pos = mixed_signal[pos_idx0];
            let y1_pos = mixed_signal[pos_idx1];

            let ym_pos = (y1_pos * self.params.diff_t
                + y0_pos * (self.params.dt - self.params.diff_t))
                / self.params.dt;
            let edge_pos = self.params.diff_t * 0.5 * (y0_pos + ym_pos);

            let li = (integ + edge_neg + edge_pos) / (2.0 * self.params.t_fil);
            out.push(li);
        }

        out
    }
}
