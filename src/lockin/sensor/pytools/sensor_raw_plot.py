import gsplot as gs
from numpy.typing import NDArray


class SensorRawPlotter:
    def __init__(self):
        pass

    def plot(self, t: NDArray, y: NDArray, index_arr: list[int], c_bg_arr: NDArray):
        ch_num = len(index_arr)
        mosaic = "".join([chr(65 + i) for i in range(ch_num)])
        axs = gs.axes(False, size=(6 * ch_num, 6), mosaic=mosaic)
        for i, (yi, c_bg) in enumerate(zip(y, c_bg_arr)):
            gs.scatter(axs[i], t * 1e6, yi)
            axs[i].axhline(c_bg, color="red", ls="--", lw=1, label="Background")

        gs.legend_axes()
        label = [["$t$ ($\\mu$s)", f"$V_{{\\rm Ch{i}}}$ (V)"] for i in index_arr]
        gs.label(label)
        gs.show()
