use std::f64::consts::PI;

#[derive(Clone, Copy)]
pub enum RefType {
    Sin,
    Cos,
}

pub struct LockinProcessor {
    t: Vec<f64>,
    data: Vec<f64>,
    f_ref: f64,
    omega_tref: f64,
    fil_length: usize,
    evr: usize,

    dt: f64,
    omega: f64,
    t_fil: f64,
    n_fil: usize,
    diff_t: f64,
    n_int: usize,
}

impl LockinProcessor {
    pub fn new(
        t: Vec<f64>,
        data: Vec<f64>,
        f_ref: f64,
        omega_tref: f64,
        fil_length: usize,
        evr: usize,
    ) -> Self {
        assert!(t.len() >= 2);
        assert_eq!(t.len(), data.len());

        let dt = t[1] - t[0];
        let omega = 2.0 * PI * f_ref;
        let t_fil = (1.0 / f_ref) * fil_length as f64;
        let n_fil = (t_fil / dt).floor() as usize;
        let diff_t = t_fil - (n_fil as f64) * dt;
        let n_int = ((t.len() - 1) / evr) + 1;

        Self {
            t,
            data,
            f_ref,
            omega_tref,
            fil_length,
            evr,
            dt,
            omega,
            t_fil,
            n_fil,
            diff_t,
            n_int,
        }
    }

    fn ref_signal(&self, t: f64, harmonic: usize, ref_type: RefType) -> f64 {
        let arg = (harmonic as f64) * (self.omega * t - self.omega_tref);
        match ref_type {
            RefType::Sin => arg.sin(),
            RefType::Cos => arg.cos(),
        }
    }

    pub fn compute_lockin(&self, harmonic: usize, ref_type: RefType) -> Vec<f64> {
        let i_start = 2 + (self.n_fil + 1) / self.evr;
        let i_end = self.n_int - i_start;
        let m = if i_end >= i_start {
            i_end - i_start + 1
        } else {
            0
        };
        let mut out = Vec::with_capacity(m);

        for k in 0..m {
            let i_idx = i_start + k;
            let i_base = i_idx * self.evr;

            // main integral
            let mut integ = 0.0;
            for j in 0..(2 * self.n_fil) {
                let j0 = j as isize - self.n_fil as isize;
                let j1 = j0 + 1;
                let idx0 = (i_base as isize + j0) as usize;
                let idx1 = (i_base as isize + j1) as usize;

                let t0 = self.t[idx0];
                let t1 = self.t[idx1];
                let f0 = self.data[idx0] * self.ref_signal(t0, harmonic, ref_type);
                let f1 = self.data[idx1] * self.ref_signal(t1, harmonic, ref_type);

                integ += 0.5 * (f0 + f1) * self.dt;
            }

            // left edge
            let neg_idx0 = i_base - self.n_fil;
            let neg_idx1 = i_base - self.n_fil - 1;
            let t_neg0 = self.t[neg_idx0];
            let t_neg1 = self.t[neg_idx1];
            let y0_neg = self.data[neg_idx0] * self.ref_signal(t_neg0, harmonic, ref_type);
            let y1_neg = self.data[neg_idx1] * self.ref_signal(t_neg1, harmonic, ref_type);
            let ym_neg = (y1_neg * self.diff_t + y0_neg * (self.dt - self.diff_t)) / self.dt;
            let edge_neg = self.diff_t * 0.5 * (y0_neg + ym_neg);

            // right edge
            let pos_idx0 = i_base + self.n_fil;
            let pos_idx1 = i_base + self.n_fil + 1;
            let t_pos0 = self.t[pos_idx0];
            let t_pos1 = self.t[pos_idx1];
            let y0_pos = self.data[pos_idx0] * self.ref_signal(t_pos0, harmonic, ref_type);
            let y1_pos = self.data[pos_idx1] * self.ref_signal(t_pos1, harmonic, ref_type);
            let ym_pos = (y1_pos * self.diff_t + y0_pos * (self.dt - self.diff_t)) / self.dt;
            let edge_pos = self.diff_t * 0.5 * (y0_pos + ym_pos);

            let li = (integ + edge_neg + edge_pos) / (2.0 * self.t_fil);
            out.push(li);
        }

        out
    }
}
