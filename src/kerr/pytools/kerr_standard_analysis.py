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
                stride = max(1, int(np.ceil(len(t) / max_points)))
                t_plot = t[::stride]
                x_plot = x[::stride]
                kerr_plot = kerr[::stride]

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
