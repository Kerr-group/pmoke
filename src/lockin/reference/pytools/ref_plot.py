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
        output_path,
    ):
        fig, axd = gs.subplots(mosaic="A", size=(6, 6), unit="in")
        axs = list(axd.values())
        gs.line(axs[0], t * 1e6, y, marker="", linestyle="-")
        gs.line(axs[0], t * 1e6, fit, color="red", ms=0, ls="--", lw=1)
        gs.label(axs[0], "$t$ ($\\mu$s)", "$V_{ref}$ (V)")
        finish_plot(output_path, interactive)
