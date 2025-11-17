import gsplot as gs
from numpy.typing import NDArray


class LIPlotter:
    def __init__(self):
        pass

    def plot(self, t: NDArray, y: NDArray, index_arr: list[int], labels: list[str]):
        ch_num = len(index_arr)
        mosaic = ";".join(
            [f"{chr(65 + 2*i)}{chr(65 + 2*i + 1)}" for i in range(ch_num)]
        )

        axs = gs.axes(False, size=(12, 6 * ch_num), mosaic=mosaic, ion=False)

        label = []
        for i, si in enumerate(y):
            # si = [LI1x, LI1y, LI2x, LI2y, ...]
            # li_odd = [LI1x, LI1y, LI3x, LI3y, ...]
            li_odd = [val for idx, val in enumerate(si) if (idx // 2) % 2 == 0]
            li_even = [val for idx, val in enumerate(si) if (idx // 2) % 2 == 1]
            label_odd = [
                labels[idx] for idx in range(len(labels)) if (idx // 2) % 2 == 0
            ]
            label_even = [
                labels[idx] for idx in range(len(labels)) if (idx // 2) % 2 == 1
            ]
            cm = gs.get_cmap("viridis", len(li_odd))

            for j, li_odd_j in enumerate(li_odd):
                gs.scatter(
                    axs[2 * i], t * 1e6, li_odd_j, label=label_odd[j], color=cm[j]
                )

            for j, li_even_j in enumerate(li_even):
                gs.scatter(
                    axs[2 * i + 1], t * 1e6, li_even_j, label=label_even[j], color=cm[j]
                )

            label_i = [
                ["$t$ ($\\mu$s)", f"Ch{index_arr[i]} : Lock-in Odd (V)"],
                ["$t$ ($\\mu$s)", f"Ch{index_arr[i]} : Lock-in Even (V)"],
            ]
            label.extend(label_i)

        gs.legend_axes(markerscale=3)
        gs.label(label)
        gs.show()
