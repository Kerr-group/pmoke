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


class PhaseRotationPlotter:
    def __init__(self):
        pass

    def plot(
        self,
        t: NDArray,
        y: NDArray,
        index_arr: list[int],
        labels: list[str],
        save: bool,
        interactive: bool,
        output_path,
    ):
        ch_num = len(index_arr)
        mosaic = ";".join(
            [f"{chr(65 + 2*i)}{chr(65 + 2*i + 1)}" for i in range(ch_num)]
        )

        axs = gs.axes(False, size=(12, 6 * ch_num), mosaic=mosaic, ion=interactive)

        label = []
        for i, si in enumerate(y):
            # si = [LI1x, LI1y, LI2x, LI2y, ...]
            # li_odd = [LI1x, LI1y, LI3x, LI3y, ...]
            li_odd = [val for idx, val in enumerate(si) if (idx // 2) % 2 == 0]
            li_even = [val for idx, val in enumerate(si) if (idx // 2) % 2 == 1]
            label_odd = [
                labels[idx] for idx in range(len(labels)) if (idx // 2) % 2 == 0
            ]
            label_even = [
                labels[idx] for idx in range(len(labels)) if (idx // 2) % 2 == 1
            ]
            cm = gs.get_cmap("viridis", len(li_odd))

            for j, li_odd_j in enumerate(li_odd):
                gs.line(
                    axs[2 * i],
                    t * 1e6,
                    li_odd_j,
                    label=label_odd[j],
                    color=cm[j],
                    marker="",
                    linestyle="-",
                )

            for j, li_even_j in enumerate(li_even):
                gs.line(
                    axs[2 * i + 1],
                    t * 1e6,
                    li_even_j,
                    label=label_even[j],
                    color=cm[j],
                    marker="",
                    linestyle="-",
                )

            label_i = [
                ["$t$ ($\\mu$s)", f"Ch{index_arr[i]} : Lock-in Odd (V)"],
                ["$t$ ($\\mu$s)", f"Ch{index_arr[i]} : Lock-in Even (V)"],
            ]
            label.extend(label_i)

        gs.legend_axes(markerscale=3)
        gs.label(label)
        finish_plot(output_path, interactive)
