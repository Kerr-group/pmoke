import lmfit
from numpy.typing import NDArray


class PulseBgFit:
    def __init__(self):
        pass

    def fit(self, t: NDArray, y: NDArray) -> dict[str, float]:
        model = lmfit.models.ConstantModel()
        params = model.make_params()
        params["c"].set(value=0, vary=True)

        result = model.fit(
            y,
            x=t,
            params=params,
            method="leastsq",
            fit_kws={
                "ftol": 1e-15,
                "xtol": 1e-15,
                "gtol": 1e-15,
            },
        )
        c = float(result.params["c"].value)
        return {"c": c}
