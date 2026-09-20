import warnings

import gsplot as gs
import lmfit
import numpy as np
from numpy.typing import NDArray

warnings.filterwarnings(
    "ignore",
    message='Creating legend with loc="best" can be slow.*',
    category=UserWarning,
)

# Liveness bound for the omega_t0 least-squares fit (single slope
# parameter: converges in a handful of evaluations on valid data).
# The fit must never iterate unboundedly on degenerate inputs; a fit
# that exhausts the budget fails closed below instead of returning a
# silent garbage slope.
FIT_MAX_NFEV = 2000


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


def decimation_indices(series, max_points: int, method: str) -> NDArray:
    length = len(series[0])
    if method == "none" or length <= max_points:
        return np.arange(length)
    if method == "stride":
        stride = max(1, int(np.ceil(length / max_points)))
        return np.arange(0, length, stride)
    if method != "min_max":
        raise ValueError(f"unknown plot decimation method: {method}")
    if max_points == 1:
        best_index = 0
        best_value = -np.inf
        for values in series:
            finite = np.flatnonzero(np.isfinite(values))
            if finite.size:
                local = finite[np.argmax(np.abs(values[finite]))]
                if abs(values[local]) > best_value:
                    best_index = local
                    best_value = abs(values[local])
        return np.array([best_index])
    bins = max(1, max_points // (2 * len(series)))
    indices = []
    for bin_index in range(bins):
        start = bin_index * length // bins
        end = max(start + 1, (bin_index + 1) * length // bins)
        for values in series:
            finite = np.flatnonzero(np.isfinite(values[start:end]))
            if finite.size == 0:
                indices.append(start)
                continue
            local = values[start:end][finite]
            indices.extend(
                (start + finite[np.argmin(local)], start + finite[np.argmax(local)])
            )
    unique = np.unique(indices)
    if unique.size <= max_points:
        return unique
    return unique[np.linspace(0, unique.size - 1, max_points, dtype=int)]


def ensure_converged(result) -> None:
    """Fail closed when the omega_t0 fit did not converge.

    A non-converged slope is not a usable phase reference: returning it
    would silently rotate every downstream harmonic by garbage, so the
    phase stage must abort with an explicit error instead.
    """
    if not result.success:
        raise RuntimeError(
            "omega_t0 fit failed to converge "
            f"(success={result.success}, nfev={result.nfev}, "
            f"message={result.message!r}); refusing to rotate by "
            "an unconverged slope"
        )


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
        output_path,
        max_points: int,
        decimation: str,
    ):
        ones = np.ones(len(m_ot0_2))
        harmonics_even = np.concatenate([ones * 2, ones * 4, ones * 6])
        m_omega_t0_even = np.concatenate([m_ot0_2, m_ot0_4, m_ot0_6])

        model = lmfit.models.LinearModel()
        params = model.make_params(intercept=0, slope=0)
        # do not vary the intercept
        params["intercept"].vary = False
        result = model.fit(
            m_omega_t0_even, params, x=harmonics_even, max_nfev=FIT_MAX_NFEV
        )
        ensure_converged(result)

        # Create data for plotting
        harmonics_even_plot = np.linspace(0, 7, 100)
        m_omega_t0_even_plot = result.eval(x=harmonics_even_plot)

        label = f"$-\\omega t_0$ = {result.params['slope'].value:.2e}$n$"

        plot_error = None
        if save or interactive:
            try:

                indices = decimation_indices(
                    [m_ot0_1, m_ot0_2, m_ot0_3, m_ot0_4, m_ot0_5, m_ot0_6],
                    max_points,
                    decimation,
                )
                ones_plot = ones[indices]

                fig, axd = gs.subplots(mosaic="A", size=(6, 6), unit="in")
                axs = list(axd.values())
                cm = gs.sample_cmap("viridis", count=6)

                gs.scatter(
                    axs[0], ones_plot * 1, m_ot0_1[indices], label="1", color=cm[0]
                )
                gs.scatter(
                    axs[0], ones_plot * 2, m_ot0_2[indices], label="2", color=cm[1]
                )
                gs.scatter(
                    axs[0], ones_plot * 3, m_ot0_3[indices], label="3", color=cm[2]
                )
                gs.scatter(
                    axs[0], ones_plot * 4, m_ot0_4[indices], label="4", color=cm[3]
                )
                gs.scatter(
                    axs[0], ones_plot * 5, m_ot0_5[indices], label="5", color=cm[4]
                )
                gs.scatter(
                    axs[0], ones_plot * 6, m_ot0_6[indices], label="6", color=cm[5]
                )

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

                gs.label(axs[0], "$n$", "$-\\omega t_0$ (rad)", xlim=[0, 7])
                finish_plot(output_path, interactive)
            except Exception as exc:
                plot_error = str(exc)

        omega_t0 = -result.params["slope"].value

        return {
            "omega_t0": omega_t0,
            "plot_error": plot_error,
            "nfev": int(result.nfev),
        }
