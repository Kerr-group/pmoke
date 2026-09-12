import numpy as np
from numpy.typing import NDArray
from scipy.special import jn


def _load_gsplot():
    import importlib
    import json
    import os
    import tempfile

    previous = os.getcwd()
    with tempfile.TemporaryDirectory(prefix="pmoke-gsplot-") as directory:
        with open(os.path.join(directory, "gsplot.json"), "w") as config:
            json.dump({"metadata": False}, config)
        os.chdir(directory)
        try:
            return importlib.import_module("gsplot")
        finally:
            os.chdir(previous)


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
        return np.array([0 if finite.size == 0 else finite[np.argmax(np.abs(values[finite]))]])
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
        indices.extend((start + finite[np.argmin(local)], start + finite[np.argmax(local)]))
    unique = np.unique(indices)
    if unique.size <= max_points:
        return unique
    return unique[np.linspace(0, unique.size - 1, max_points, dtype=int)]


class MokeStandardAnalyser:
    #: Shared modulation depth with the angle computation (D8).
    DEFAULT_PHIM = 0.92
    #: Bessel denominators at or below this magnitude count as degenerate,
    #: mirroring BESSEL_DENOMINATOR_MIN in pmoke-analysis-core (FR-13).
    BESSEL_DENOMINATOR_MIN = 1e-12

    def __init__(self):
        pass

    @staticmethod
    def calculate(a1: NDArray, a2: NDArray, phim=DEFAULT_PHIM) -> NDArray:
        frac_top = jn(2, 2 * phim) * a1
        frac_bottom = jn(1, 2 * phim) * a2
        with np.errstate(divide="ignore", invalid="ignore"):
            ratio = np.divide(frac_top, frac_bottom)
        return (1 / 2) * np.arctan(ratio)

    @staticmethod
    def calculate_vm(a1: NDArray, a2: NDArray, phim=DEFAULT_PHIM) -> NDArray:
        a1 = np.asarray(a1, dtype=float)
        a2 = np.asarray(a2, dtype=float)
        if not np.isfinite(phim):
            raise ValueError("phim must be finite")
        if not np.all(np.isfinite(a1)) or not np.all(np.isfinite(a2)):
            raise ValueError("harmonic inputs must be finite")
        denominators = np.array([jn(1, 2 * phim), jn(2, 2 * phim)])
        if not np.all(np.isfinite(denominators)) or np.any(
            np.abs(denominators) <= MokeStandardAnalyser.BESSEL_DENOMINATOR_MIN
        ):
            raise ValueError("Bessel denominators must be finite and nonzero")
        return 0.5 * np.sqrt((a1 / denominators[0]) ** 2 + (a2 / denominators[1]) ** 2)

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

        angle = factor * self.calculate(li1_in, li2_in)
        vm = self.calculate_vm(li1_in, li2_in)

        plot_error = None
        if save or interactive:
            try:
                gs = _load_gsplot()

                indices = decimation_indices(angle, max_points, decimation)
                t_plot = t[indices]
                x_plot = x[indices]
                angle_plot = angle[indices]
                vm_plot = vm[indices]

                axs = gs.axes(
                    True,
                    size=(12, 6),
                    mosaic="AB",
                    ion=interactive,
                )

                gs.scatter_colormap(axs[0], x_plot, angle_plot * 1e3, t_plot)
                gs.scatter_colormap(axs[1], x_plot, vm_plot * 1e3, t_plot)
                axs[0].grid()

                title = fig_name + " using Standard"
                gs.title(title)

                gs.label(
                    [
                        [f"{xlabel}", "$\\theta_{\\rm K}$ (mrad)"],
                        [f"{xlabel}", "$V_{\\rm m}$ (mV)"],
                    ]
                )
                finish_plot(output_path, interactive)
            except Exception as exc:
                plot_error = str(exc)

        return {
            "angle": angle,
            "vm": vm,
            "plot_error": plot_error,
        }
