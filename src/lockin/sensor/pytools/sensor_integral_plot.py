import gsplot as gs
from numpy.typing import NDArray


def finish_plot(fname: str, save: bool, interactive: bool):
    if interactive:
        import matplotlib.pyplot as plt

        plt.ioff()
        if save:
            plt.savefig(f"{fname}.png", bbox_inches="tight")
        plt.show(block=True)
        plt.close("all")
    elif save:
        gs.show(fname, ft_list=["png"], show=False)


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
    ):
        ch_num = len(index_arr)
        mosaic = "".join([chr(65 + i) for i in range(ch_num)])
        axs = gs.axes(False, size=(6 * ch_num, 6), mosaic=mosaic, ion=interactive)
        for i, yi in enumerate(y):
            gs.line(axs[i], t * 1e6, yi, marker="", linestyle="-")

        label = [
            ["$t$ ($\\mu$s)", f"Ch {index_arr[i]} : {label_arr[i]} ({unit_arr[i]})"]
            for i in range(ch_num)
        ]
        gs.label(label)
        finish_plot("sensor_integral", save, interactive)
