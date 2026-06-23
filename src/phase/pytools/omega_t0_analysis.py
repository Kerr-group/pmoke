import gsplot as gs
import lmfit
import numpy as np
import warnings
from numpy.typing import NDArray

warnings.filterwarnings(
    "ignore",
    message='Creating legend with loc="best" can be slow.*',
    category=UserWarning,
)


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


class OT0Analyser:
    def __init__(self):
        pass

    def analyse(
        self,
        m_ot0_1: NDArray,
        m_ot0_2: NDArray,
        m_ot0_3: NDArray,
        m_ot0_4: NDArray,
        m_ot0_5: NDArray,
        m_ot0_6: NDArray,
        save: bool,
        interactive: bool,
        max_points: int,
    ):
        ones = np.ones(len(m_ot0_2))
        harmonics_even = np.concatenate([ones * 2, ones * 4, ones * 6])
        m_omega_t0_even = np.concatenate([m_ot0_2, m_ot0_4, m_ot0_6])

        model = lmfit.models.LinearModel()
        params = model.make_params(intercept=0, slope=0)
        # do not vary the intercept
        params["intercept"].vary = False
        result = model.fit(m_omega_t0_even, params, x=harmonics_even)

        # Create data for plotting
        harmonics_even_plot = np.linspace(0, 7, 100)
        m_omega_t0_even_plot = result.eval(x=harmonics_even_plot)

        label = f"$-\\omega t_0$ = {result.params['slope'].value:.2e}$n$"

        plot_error = None
        if save or interactive:
            try:
                stride = max(1, int(np.ceil(len(ones) / max_points)))
                ones_plot = ones[::stride]

                axs = gs.axes(False, size=(6, 6), mosaic="A", ion=interactive)
                cm = gs.get_cmap(cmap="viridis", N=6)

                gs.scatter(axs[0], ones_plot * 1, m_ot0_1[::stride], label="1", color=cm[0])
                gs.scatter(axs[0], ones_plot * 2, m_ot0_2[::stride], label="2", color=cm[1])
                gs.scatter(axs[0], ones_plot * 3, m_ot0_3[::stride], label="3", color=cm[2])
                gs.scatter(axs[0], ones_plot * 4, m_ot0_4[::stride], label="4", color=cm[3])
                gs.scatter(axs[0], ones_plot * 5, m_ot0_5[::stride], label="5", color=cm[4])
                gs.scatter(axs[0], ones_plot * 6, m_ot0_6[::stride], label="6", color=cm[5])

                gs.line(
                    axs[0],
                    harmonics_even_plot,
                    m_omega_t0_even_plot,
                    label=label,
                    ms=0,
                    ls="--",
                    color="red",
                )

                gs.legend(axs[0], loc="best", markerscale=5)

                gs.label([["$n$", "$-\\omega t_0$ (rad)", [0, 7], ["", ""]]])
                finish_plot("omega_t0_analysis", save, interactive)
            except Exception as exc:
                plot_error = str(exc)

        omega_t0 = -result.params["slope"].value

        return {
            "omega_t0": omega_t0,
            "plot_error": plot_error,
        }
