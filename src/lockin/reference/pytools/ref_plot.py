import gsplot as gs
from numpy.typing import NDArray


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
        if save:
            gs.show("reference_fit", ft_list=["png"], show=interactive)
        elif interactive:
            import matplotlib.pyplot as plt

            plt.show()
