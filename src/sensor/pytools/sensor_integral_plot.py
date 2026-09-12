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


class SensorIntegralPlotter:
    def __init__(self):
        pass

    def plot(
        self,
        t: NDArray,
        y: NDArray,
        index_arr: list[int],
        label_arr: list[str],
        unit_arr: list[str],
        save: bool,
        interactive: bool,
        output_path,
    ):
        ch_num = len(index_arr)
        mosaic = "".join([chr(65 + i) for i in range(ch_num)])
        fig, axd = gs.subplots(mosaic=mosaic, size=(6 * ch_num, 6), unit="in")
        axs = list(axd.values())
        for i, yi in enumerate(y):
            gs.line(axs[i], t * 1e6, yi, marker="", linestyle="-")
        label = [
            ["$t$ ($\\mu$s)", f"Ch {index_arr[i]} : {label_arr[i]} ({unit_arr[i]})"]
            for i in range(ch_num)
        ]
        for ax, record in zip(axs, label):
            gs.label(ax, record[0], record[1])
            ax.grid()
        finish_plot(output_path, interactive)
