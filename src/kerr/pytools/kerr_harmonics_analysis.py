import gsplot as gs
import numpy as np
from numpy.typing import NDArray
from scipy.special import jn


class KerrHarmonicsAnalyser:
    def __init__(self):
        pass

    @staticmethod
    def calculate(a1: NDArray, a2: NDArray, phim=0.92) -> NDArray:
        frac_top = jn(2, 2 * phim) * a1
        frac_bottom = jn(1, 2 * phim) * a2
        return (1 / 2) * np.arctan(frac_top / frac_bottom)

    @staticmethod
    def get_modulation_depth(a2: NDArray, a4: NDArray, a6: NDArray) -> NDArray:
        return 6 * np.sqrt(20 * a4 / (15 * a2 + 24 * a4 + 9 * a6))

    @staticmethod
    def get_kerr(x0: float, a2: NDArray, a3: NDArray, a4: NDArray) -> NDArray:
        return (a3 / (a2 + a4)) * (3.0 / x0)

    def analyse(
        self,
        t: NDArray,
        x: NDArray,
        ys: NDArray,
        factor: float,
        xlabel: str,
        fig_name: str,
    ):

        li1_in, li1_out = ys[0], ys[1]
        li2_in, li2_out = ys[2], ys[3]
        li3_in, li3_out = ys[4], ys[5]
        li4_in, li4_out = ys[6], ys[7]
        li5_in, li5_out = ys[8], ys[9]
        li6_in, li6_out = ys[10], ys[11]

        x0 = self.get_modulation_depth(li2_in, li4_in, li6_in)
        mean_x0 = float(np.mean(x0))

        kerr = self.get_kerr(mean_x0, li2_in, li3_in, li4_in)
        kerr = kerr * factor

        axs = gs.axes(
            True,
            size=(6, 6),
            mosaic="A",
            ion=False,
        )

        gs.scatter_colormap(axs[0], x, kerr * 1e3, t)
        axs[0].grid()
        title = fig_name + " using Harmonics"
        gs.title(title)

        gs.label([[f"{xlabel}", "$\\theta_{\\rm K}$ (mrad)"]])
        gs.show(fig_name, ft_list=["png"])

        return {
            "kerr": kerr,
        }
