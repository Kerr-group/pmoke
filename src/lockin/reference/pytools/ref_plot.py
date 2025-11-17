import gsplot as gs
from numpy.typing import NDArray


class ReferencePlotter:
    def __init__(self):
        pass

    def plot(self, t: NDArray, y: NDArray, fit: NDArray):
        axs = gs.axes(False, size=(6, 6), mosaic="A", ion=False)
        gs.scatter(axs[0], t * 1e6, y)
        gs.line(axs[0], t * 1e6, fit, color="red", ms=0, ls="--", lw=1)
        gs.label([["$t$ ($\\mu$s)", "$V_{ref}$ (V)"]])
        gs.show()
