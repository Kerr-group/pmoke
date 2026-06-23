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


class ReferencePlotter:
    def __init__(self):
        pass

    def plot(
        self,
        t: NDArray,
        y: NDArray,
        fit: NDArray,
        save: bool,
        interactive: bool,
    ):
        axs = gs.axes(False, size=(6, 6), mosaic="A", ion=interactive)
        gs.line(axs[0], t * 1e6, y, marker="", linestyle="-")
        gs.line(axs[0], t * 1e6, fit, color="red", ms=0, ls="--", lw=1)
        gs.label([["$t$ ($\\mu$s)", "$V_{ref}$ (V)"]])
        finish_plot("reference_fit", save, interactive)
