from typing import Union

import numpy as np
from numpy.typing import NDArray
from scipy.special import jn
import gsplot as gs

def finish_plot(output_path, interactive: bool):
    import matplotlib.pyplot as plt

    if interactive:
        plt.ioff()
        if output_path is not None:
            plt.savefig(output_path, bbox_inches="tight")
        plt.show(block=True)
        plt.close("all")
    elif output_path is not None:
        plt.savefig(output_path, bbox_inches="tight")
        plt.close("all")

def decimation_indices(values: NDArray, max_points: int, method: str) -> NDArray:
    length = len(values)
    if method == "none" or length <= max_points:
        return np.arange(length)
    if method == "stride":
        stride = max(1, int(np.ceil(length / max_points)))
        return np.arange(0, length, stride)
    if method != "min_max":
        raise ValueError(f"unknown plot decimation method: {method}")
    if max_points == 1:
        finite = np.flatnonzero(np.isfinite(values))
        return np.array(
            [0 if finite.size == 0 else finite[np.argmax(np.abs(values[finite]))]]
        )
    bins = max(1, max_points // 2)
    indices = []
    for bin_index in range(bins):
        start = bin_index * length // bins
        end = max(start + 1, (bin_index + 1) * length // bins)
        finite = np.flatnonzero(np.isfinite(values[start:end]))
        if finite.size == 0:
            indices.append(start)
            continue
        local = values[start:end][finite]
        indices.extend(
            (start + finite[np.argmin(local)], start + finite[np.argmax(local)])
        )
    unique = np.unique(indices)
    if unique.size <= max_points:
        return unique
    return unique[np.linspace(0, unique.size - 1, max_points, dtype=int)]

class MokeHarmonicsAnalyser:
    def __init__(self):
        pass

    @staticmethod
    def calculate(a1: NDArray, a2: NDArray, phim=0.92) -> NDArray:
        frac_top = jn(2, 2 * phim) * a1
        frac_bottom = jn(1, 2 * phim) * a2
        return (1 / 2) * np.arctan(frac_top / frac_bottom)

    @staticmethod
    def get_modulation_depth(a2: NDArray, a4: NDArray, a6: NDArray) -> NDArray:
        denominator = 15 * a2 + 24 * a4 + 9 * a6
        with np.errstate(divide="ignore", invalid="ignore"):
            return 6 * np.sqrt(np.divide(20 * a4, denominator))

    @staticmethod
    def get_representative_modulation_depth(x0: NDArray) -> float:
        valid_x0 = x0[np.isfinite(x0) & (x0 > 0)]
        if valid_x0.size == 0:
            raise ValueError("cannot determine a finite positive modulation depth")
        return float(np.median(valid_x0))

    @staticmethod
    def get_moke(
        x0: Union[float, NDArray], a2: NDArray, a3: NDArray, a4: NDArray
    ) -> NDArray:
        denominator = (a2 + a4) * x0 / 6
        with np.errstate(divide="ignore", invalid="ignore"):
            ratio = np.divide(a3, denominator)
        return 0.5 * np.arctan(ratio)

    #: Shared guard with calculate_harmonics_vm in pmoke-analysis-core (FR-13).
    BESSEL_DENOMINATOR_MIN = 1e-12

    @staticmethod
    def get_vm(x0: float, a2: NDArray, a3: NDArray) -> NDArray:
        a2 = np.asarray(a2, dtype=float)
        a3 = np.asarray(a3, dtype=float)
        if not np.isfinite(x0):
            raise ValueError("modulation depth must be finite")
        if not np.all(np.isfinite(a2)) or not np.all(np.isfinite(a3)):
            raise ValueError("harmonic inputs must be finite")
        denominators = np.array([jn(2, x0), jn(3, x0)])
        if not np.all(np.isfinite(denominators)) or np.any(
            np.abs(denominators) <= MokeHarmonicsAnalyser.BESSEL_DENOMINATOR_MIN
        ):
            raise ValueError("Bessel denominators must be finite and nonzero")
        return 0.5 * np.sqrt((a3 / denominators[1]) ** 2 + (a2 / denominators[0]) ** 2)

    def analyse(
        self,
        t: NDArray,
        x: NDArray,
        ys: NDArray,
        factor: float,
        xlabel: str,
        fig_name: str,
        save: bool,
        interactive: bool,
        output_path,
        vm_output_path,
        max_points: int,
        decimation: str,
    ):

        li1_in, li1_out = ys[0], ys[1]
        li2_in, li2_out = ys[2], ys[3]
        li3_in, li3_out = ys[4], ys[5]
        li4_in, li4_out = ys[6], ys[7]
        li5_in, li5_out = ys[8], ys[9]
        li6_in, li6_out = ys[10], ys[11]

        x0_series = self.get_modulation_depth(li2_in, li4_in, li6_in)
        x0 = self.get_representative_modulation_depth(x0_series)

        moke = self.get_moke(x0, li2_in, li3_in, li4_in)
        moke = moke * factor
        vm = self.get_vm(x0, li2_in, li3_in)

        plot_error = self.plot(
            t,
            x,
            moke,
            vm,
            xlabel,
            fig_name,
            save,
            interactive,
            output_path,
            vm_output_path,
            max_points,
            decimation,
        )

        return {
            "moke": moke,
            "vm": vm,
            "plot_error": plot_error,
        }

    @staticmethod
    def plot(
        t: NDArray,
        x: NDArray,
        moke: NDArray,
        vm: NDArray,
        xlabel: str,
        fig_name: str,
        save: bool,
        interactive: bool,
        output_path,
        vm_output_path,
        max_points: int,
        decimation: str,
    ):
        if not (save or interactive):
            return None
        try:
            indices = decimation_indices(moke, max_points, decimation)
            t_plot = t[indices]
            x_plot = x[indices]
            moke_plot = moke[indices]
            vm_plot = vm[indices]

            fig, axd = gs.subplots(mosaic="AB", size=(12, 6), unit="in")
            axs = list(axd.values())

            gs.cmap_scatter(axs[0], x_plot, moke_plot * 1e3, t_plot)
            gs.cmap_scatter(axs[1], x_plot, vm_plot * 1e3, t_plot)
            axs[0].grid()
            title = fig_name + " using Harmonics"
            gs.suptitle(fig, title)

            gs.label(axs[0], xlabel, "$\\theta_{\\rm K}$ (mrad)")
            gs.label(axs[1], xlabel, "$V_{\\rm m}$ (mV)")
            finish_plot(output_path, interactive)

            return None
        except Exception as exc:
            return str(exc)
