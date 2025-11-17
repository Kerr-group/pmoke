import gsplot as gs
import lmfit
import numpy as np
from numpy.typing import NDArray


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
    ):
        ones = np.ones(len(m_ot0_2))
        harmonics_even = np.concatenate([ones * 2, ones * 4, ones * 6])
        m_omega_t0_even = np.concatenate([m_ot0_2, m_ot0_4, m_ot0_6])

        model = lmfit.models.LinearModel()
        params = model.make_params(intercept=0, slope=0)
        # do not vary the intercept
        params["intercept"].vary = False
        result = model.fit(m_omega_t0_even, params, x=harmonics_even)
        print("🛠️ Fit result:")
        print(result.fit_report())

        # Create data for plotting
        harmonics_even_plot = np.linspace(0, 7, 100)
        m_omega_t0_even_plot = result.eval(x=harmonics_even_plot)

        label = f"$-\\omega t_0$ = {result.params['slope'].value:.2e}$n$"

        axs = gs.axes(False, size=(6, 6), mosaic="A", ion=False)
        cm = gs.get_cmap(cmap="viridis", N=6)

        gs.scatter(axs[0], ones * 1, m_ot0_1, label="1", color=cm[0])
        gs.scatter(axs[0], ones * 2, m_ot0_2, label="2", color=cm[1])
        gs.scatter(axs[0], ones * 3, m_ot0_3, label="3", color=cm[2])
        gs.scatter(axs[0], ones * 4, m_ot0_4, label="4", color=cm[3])
        gs.scatter(axs[0], ones * 5, m_ot0_5, label="5", color=cm[4])
        gs.scatter(axs[0], ones * 6, m_ot0_6, label="6", color=cm[5])

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
        gs.show()

        omega_t0 = -result.params["slope"].value

        return {
            "omega_t0": omega_t0,
        }
