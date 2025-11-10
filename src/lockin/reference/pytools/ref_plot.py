import gsplot as gs

class ReferencePlotter:
    def __init__(self):
        pass

    def plot(self, t, y, fit):
        axs = gs.axes(False, size=(6,6), mosaic="A")
        gs.scatter(axs[0], t*1e6, y)
        gs.line(axs[0], t*1e6, fit, color='red', ms = 0, ls = "--", lw = 1)
        gs.label([
            ["$t$ (µs)", "$V_{ref}$ (V)"]
        ])
        gs.show()
