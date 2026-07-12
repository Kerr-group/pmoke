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


gs = _load_gsplot()
import warnings
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
        axs = gs.axes(False, size=(6 * ch_num, 6), mosaic=mosaic, ion=interactive)
        for i, (yi, c_bg) in enumerate(zip(y, c_bg_arr)):
            gs.line(axs[i], t * 1e6, yi, marker="", linestyle="-")
            axs[i].axhline(c_bg, color="red", ls="--", lw=1, label="Background")

        gs.legend_axes()
        label = [["$t$ ($\\mu$s)", f"$V_{{\\rm Ch{i}}}$ (V)"] for i in index_arr]
        gs.label(label)
        finish_plot(output_path, interactive)
