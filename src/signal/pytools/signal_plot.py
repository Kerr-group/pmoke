import gsplot as gs
from numpy.typing import NDArray


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


class SignalPlotter:
    def __init__(self):
        pass

    def plot(
        self,
        t: NDArray,
        y: NDArray,
        labels: list,
        units: list,
        save: bool,
        interactive: bool,
        output_path,
    ):
        ch_num = len(y)
        mosaic = "".join([chr(65 + i) for i in range(ch_num)])
        fig, axd = gs.subplots(mosaic=mosaic, size=(6 * ch_num, 6), unit="in")
        axs = list(axd.values())
        for i, yi in enumerate(y):
            gs.line(axs[i], t * 1e6, yi, marker="", linestyle="-")
            axs[i].grid()
        for ax, label, unit in zip(axs, labels, units):
            gs.label(ax, "$t$ ($\\mu$s)", f"{label} ({unit})")
        finish_plot(output_path, interactive)
