import warnings

import gsplot as gs
from numpy.typing import NDArray

warnings.filterwarnings(
    "ignore",
    message='Creating legend with loc="best" can be slow.*',
    category=UserWarning,
)


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


class SensorRawPlotter:
    def __init__(self):
        pass

    def plot(
        self,
        t: NDArray,
        y: NDArray,
        index_arr: list[int],
        c_bg_arr: NDArray,
        save: bool,
        interactive: bool,
        output_path,
    ):
        ch_num = len(index_arr)
        mosaic = "".join([chr(65 + i) for i in range(ch_num)])
        fig, axd = gs.subplots(mosaic=mosaic, size=(6 * ch_num, 6), unit="in")
        axs = list(axd.values())
        for i, (yi, c_bg) in enumerate(zip(y, c_bg_arr)):
            gs.line(axs[i], t * 1e6, yi, marker="", linestyle="-")
            axs[i].axhline(c_bg, color="red", ls="--", lw=1, label="Background")
        gs.legend(axs)
        label = [["$t$ ($\\mu$s)", f"$V_{{\\rm Ch{i}}}$ (V)"] for i in index_arr]
        for ax, record in zip(axs, label):
            gs.label(ax, record[0], record[1])
        finish_plot(output_path, interactive)
