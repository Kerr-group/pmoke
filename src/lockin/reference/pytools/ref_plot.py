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
        axs = gs.axes(False, size=(6, 6), mosaic="A", ion=interactive)
        gs.line(axs[0], t * 1e6, y, marker="", linestyle="-")
        gs.line(axs[0], t * 1e6, fit, color="red", ms=0, ls="--", lw=1)
        gs.label([["$t$ ($\\mu$s)", "$V_{ref}$ (V)"]])
        finish_plot(output_path, interactive)
