import gsplot as gs
import warnings
from numpy.typing import NDArray

warnings.filterwarnings(
    "ignore",
    message='Creating legend with loc="best" can be slow.*',
    category=UserWarning,
)


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
        output_dir: str,
    ):
        ch_num = len(index_arr)
        mosaic = "".join([chr(65 + i) for i in range(ch_num)])
        axs = gs.axes(False, size=(6 * ch_num, 6), mosaic=mosaic, ion=interactive)
        for i, (yi, c_bg) in enumerate(zip(y, c_bg_arr)):
            gs.line(axs[i], t * 1e6, yi, marker="", linestyle="-")
            axs[i].axhline(c_bg, color="red", ls="--", lw=1, label="Background")

        gs.legend_axes()
        label = [["$t$ ($\\mu$s)", f"$V_{{\\rm Ch{i}}}$ (V)"] for i in index_arr]
        gs.label(label)
        finish_plot("sensor_raw", save, interactive, output_dir)
