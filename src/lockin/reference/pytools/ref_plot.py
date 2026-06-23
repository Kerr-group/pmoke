import gsplot as gs
from numpy.typing import NDArray


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
        output_dir: str,
    ):
        axs = gs.axes(False, size=(6, 6), mosaic="A", ion=interactive)
        gs.line(axs[0], t * 1e6, y, marker="", linestyle="-")
        gs.line(axs[0], t * 1e6, fit, color="red", ms=0, ls="--", lw=1)
        gs.label([["$t$ ($\\mu$s)", "$V_{ref}$ (V)"]])
        finish_plot("reference_fit", save, interactive, output_dir)
