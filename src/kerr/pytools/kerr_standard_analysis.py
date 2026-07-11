import gsplot as gs
import numpy as np
from numpy.typing import NDArray
from scipy.special import jn


def finish_plot(fname: str, save: bool, interactive: bool, output_dir: str):
    if interactive:
        import matplotlib.pyplot as plt

        plt.ioff()
        if save:
            import os

            os.makedirs(output_dir, exist_ok=True)
            path = os.path.join(output_dir, fname)
            plt.savefig(f"{path}.png", bbox_inches="tight")
        plt.show(block=True)
        plt.close("all")
    elif save:
        import os

        os.makedirs(output_dir, exist_ok=True)
        path = os.path.join(output_dir, fname)
        gs.show(path, ft_list=["png"], show=False)


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


class KerrStandardAnalyser:
    def __init__(self):
        pass

    @staticmethod
    def calculate(a1: NDArray, a2: NDArray, phim=0.92) -> NDArray:
        frac_top = jn(2, 2 * phim) * a1
        frac_bottom = jn(1, 2 * phim) * a2
        with np.errstate(divide="ignore", invalid="ignore"):
            ratio = np.divide(frac_top, frac_bottom)
        return (1 / 2) * np.arctan(ratio)

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
        output_dir: str,
        max_points: int,
        decimation: str,
    ):

        li1_in, li1_out = ys[0], ys[1]
        li2_in, li2_out = ys[2], ys[3]
        li3_in, li3_out = ys[4], ys[5]
        li4_in, li4_out = ys[6], ys[7]
        li5_in, li5_out = ys[8], ys[9]
        li6_in, li6_out = ys[10], ys[11]

        kerr = factor * self.calculate(li1_in, li2_in)

        plot_error = None
        if save or interactive:
            try:
                indices = decimation_indices(kerr, max_points, decimation)
                t_plot = t[indices]
                x_plot = x[indices]
                kerr_plot = kerr[indices]

                axs = gs.axes(
                    True,
                    size=(6, 6),
                    mosaic="A",
                    ion=interactive,
                )

                gs.scatter_colormap(axs[0], x_plot, kerr_plot * 1e3, t_plot)
                axs[0].grid()

                title = fig_name + " using Standard"
                gs.title(title)

                gs.label([[f"{xlabel}", "$\\theta_{\\rm K}$ (mrad)"]])
                finish_plot(fig_name, save, interactive, output_dir)
            except Exception as exc:
                plot_error = str(exc)

        return {
            "kerr": kerr,
            "plot_error": plot_error,
        }
