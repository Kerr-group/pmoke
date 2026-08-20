import lmfit
import numpy as np
from numpy.typing import NDArray
from scipy.signal import windows


class PreciseFFT:
    """
    1) Apply a Hann window to the signal
    2) Perform zero-padding to increase frequency resolution
    3) Refine peak detection using quadratic interpolation (complex)
    4) Extract amplitude and phase for the target frequency
    5) Compute the DC component
    """

    def __init__(self, dt: float, y: NDArray, pad_factor: int = 5):
        self.y = y
        self.pad_factor = pad_factor

        self.N = len(y)
        self.dt = dt
        self.fs = 1 / self.dt  # Sampling frequency

        self.apply_hann_window()
        self.zero_padding()

    def apply_hann_window(self):
        """Apply a Hann window to the raw signal."""
        self.window = windows.hann(self.N)
        self.y_win = self.y * self.window

    def zero_padding(self):
        """Zero-pad the windowed signal to increase resolution."""
        self.y_pad = np.pad(self.y_win, (0, self.N * (self.pad_factor - 1)), "constant")
        self.N_pad = len(self.y_pad)
        self.freq = np.fft.rfftfreq(self.N_pad, self.dt)
        self.Y = np.fft.rfft(self.y_pad)

    def quad_interp_complex(self, fft_arr, idx):
        """Complex quadratic interpolation around a given bin."""
        if idx < 1 or idx >= len(fft_arr) - 1:
            return float(idx), np.abs(fft_arr[idx]), np.angle(fft_arr[idx])

        Xm1, X0, Xp1 = fft_arr[idx - 1], fft_arr[idx], fft_arr[idx + 1]
        y_m1, y_0, y_p1 = np.abs(Xm1), np.abs(X0), np.abs(Xp1)

        if not (y_0 >= y_m1 and y_0 >= y_p1):
            return float(idx), y_0, np.angle(X0)

        denom = y_m1 - 2 * y_0 + y_p1
        if abs(denom) < 1e-12:
            return float(idx), y_0, np.angle(X0)

        offset = 0.5 * (y_m1 - y_p1) / denom
        offset = offset if abs(offset) <= 1 else 0.0
        peak_pos = idx + offset

        x = np.array([-1.0, 0.0, 1.0])
        poly_re = np.polyfit(x, [Xm1.real, X0.real, Xp1.real], 2)
        poly_im = np.polyfit(x, [Xm1.imag, X0.imag, Xp1.imag], 2)
        re_interp = np.polyval(poly_re, offset)
        im_interp = np.polyval(poly_im, offset)

        amp_est = np.hypot(re_interp, im_interp)
        phase_est = np.arctan2(im_interp, re_interp)
        return peak_pos, amp_est, phase_est

    def quad_interp(self, target_omega: float):
        """Refine peak around target angular frequency."""
        f_target = target_omega / (2 * np.pi)
        idx = np.argmin(np.abs(self.freq - f_target))
        peak_pos, amp_est, phase_est = self.quad_interp_complex(self.Y, idx)

        f_refined = np.interp(
            peak_pos,
            [np.floor(peak_pos), np.ceil(peak_pos)],
            [self.freq[int(np.floor(peak_pos))], self.freq[int(np.ceil(peak_pos))]],
        )
        self.freq_refined = f_refined * 2 * np.pi

        win_sum = self.window.sum()
        # Normalize by sum(window) to correct amplitude loss
        self.amp = 2 * amp_est / win_sum
        self.phase = phase_est

    def get_target_freq_component(self, target_omega: float):
        """Return amplitude and phase at the target frequency."""
        self.quad_interp(target_omega)
        return self.amp, self.phase

    def get_dc_component(self):
        """Compute DC component corrected for window."""
        win_sum = self.window.sum()
        self.dc_component = np.abs(self.Y[0]) / win_sum
        return self.dc_component

    def get_dc_component_from_sum(self):
        """Compute DC via time-domain sum approach."""
        return np.sum(self.y) / (2 * self.N_pad)

    def get_data(self):
        """
        Return one-sided amplitude spectrum:
        - omega [rad/s]
        - amp  amplitude corrected by window sum
        """
        win_sum = self.window.sum()
        amp = 2 * np.abs(self.Y) / win_sum

        omega = self.freq * 2 * np.pi
        return omega, amp


class ReferenceFFT:
    def __init__(self):
        pass

    def fft(self, dt: float, y: NDArray, pad_factor: int = 3):
        y = np.asarray(y, dtype=float)
        if y.ndim != 1 or y.size < 2:
            raise ValueError(
                "reference FFT requires a one-dimensional signal with at least two samples"
            )
        if not np.isfinite(dt) or dt <= 0:
            raise ValueError(f"reference FFT dt must be positive and finite (got {dt})")
        if not np.all(np.isfinite(y)):
            raise ValueError("reference FFT requires finite samples")

        fft = PreciseFFT(dt, y, pad_factor=pad_factor)
        centered = y - np.mean(y)
        signal_scale = float(np.max(np.abs(y)))
        signal_range = float(np.ptp(y))
        if not np.isfinite(signal_range) or signal_range <= np.finfo(float).eps * max(
            signal_scale, np.finfo(float).tiny
        ):
            raise ValueError("reference signal has no non-DC component")
        centered_scale = float(np.max(np.abs(centered)))
        if not np.isfinite(centered_scale) or centered_scale == 0.0:
            raise ValueError("reference signal has no non-DC component")

        carrier_fft = PreciseFFT(dt, centered, pad_factor=pad_factor)
        omega, fft_data = carrier_fft.get_data()
        freq = omega / (2 * np.pi)

        if len(fft_data) < 2:
            raise ValueError("reference FFT has no non-DC frequency bins")
        idx = 1 + int(np.argmax(fft_data[1:]))
        peak_amplitude = float(fft_data[idx])
        if not np.isfinite(peak_amplitude) or peak_amplitude <= (
            np.finfo(float).eps * centered_scale
        ):
            raise ValueError("reference FFT has no resolvable non-DC carrier")
        f_ref = float(freq[idx])
        if not np.isfinite(f_ref) or f_ref <= 0.0:
            raise ValueError(f"reference FFT estimated an invalid carrier frequency: {f_ref}")

        A_ref, theta_ref = fft.get_target_freq_component(2 * np.pi * f_ref)
        A_ref = float(A_ref)
        theta_ref = float(theta_ref)

        omega_tref = -(theta_ref + np.pi / 2)

        return {
            "f_ref": f_ref,
            "A_ref": A_ref,
            "omega_tref": omega_tref,
        }


class ReferenceFitter:
    def __init__(self):
        pass

    def fit(
        self,
        t: NDArray,
        y: NDArray,
        f_ref: float,
        A_ref: float,
        omega_tref: float,
    ):
        t = np.asarray(t)
        y = np.asarray(y)

        def ref_model(t, A_ref, df, omega_tref):
            return A_ref * np.sin(2 * np.pi * (f_ref + df) * t - omega_tref)

        model = lmfit.Model(ref_model)
        params = model.make_params()
        params["A_ref"].set(value=A_ref, min=A_ref * 0.5, max=A_ref * 2.0)
        params["df"].set(value=0.0, min=-100, max=100)
        params["omega_tref"].set(
            value=omega_tref, min=omega_tref - np.pi, max=omega_tref + np.pi
        )

        result = model.fit(y, t=t, params=params, method="least_squares")

        p = result.params
        df = float(p["df"].value)
        A_ref_fit = float(p["A_ref"].value)
        omega_tref_fit = float(p["omega_tref"].value)
        f_ref_fit = f_ref + df

        return {
            "f_ref": f_ref_fit,
            "A_ref": A_ref_fit,
            "omega_tref": omega_tref_fit,
        }
