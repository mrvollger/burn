use crate::{
    self as burn, grad_clipping::GradientClippingConfig, module::ADModule, record::Record,
    LearningRate,
};

use super::{
    decay::{WeightDecay, WeightDecayConfig},
    SimpleOptimizer,
};
use crate::config::Config;
use crate::optim::adaptor::OptimizerAdaptor;
use crate::tensor::{backend::ADBackend, Tensor};
use burn_tensor::backend::Backend;

/// Configuration to create the [RMSProp](RMSProp) optimizer.
#[derive(Config)]
pub struct RMSPropConfig {
    /// Smoothing constant.
    #[config(default = 0.99)]
    alpha: f32,
    /// momentum for RMSProp.
    #[config(default = 0.9)]
    momentum: f32,
    /// A value required for numerical stability.
    #[config(default = 1e-5)]
    epsilon: f32,
    /// if True, compute the centered RMSProp, the gradient is normalized by an estimation of its variance
    #[config(default = false)]
    centered: bool,
    /// [Weight decay](WeightDecayConfig) config.
    weight_decay: Option<WeightDecayConfig>,
    /// [Gradient Clipping](GradientClippingConfig) config.
    grad_clipping: Option<GradientClippingConfig>,
}

impl RMSPropConfig {
    /// Initialize RMSProp optimizer.
    ///
    /// # Returns
    ///
    /// Returns an optimizer that can be used to optimize a module.
    pub fn init<B: ADBackend, M: ADModule<B>>(
        &self,
    ) -> OptimizerAdaptor<RMSProp<B::InnerBackend>, M, B> {
        let weight_decay = self.weight_decay.as_ref().map(WeightDecay::new);

        let mut optim = OptimizerAdaptor::from(RMSProp {
            alpha: self.alpha,
            centered: self.centered,
            weight_decay,
            momentum: RMSPropMomentum {
                momentum: self.momentum,
                epsilon: self.epsilon,
            },
        });

        if let Some(config) = &self.grad_clipping {
            optim = optim.with_grad_clipping(config.init());
        }

        optim
    }
}

/// Optimizer that implements stochastic gradient descent with momentum.
/// The optimizer can be configured with [RMSPropConfig](RMSPropConfig).
pub struct RMSProp<B: Backend> {
    alpha: f32,
    // epsilon: f32,
    centered: bool,
    // momentum: Option<Momentum<B>>,
    momentum: RMSPropMomentum,
    weight_decay: Option<WeightDecay<B>>,
}

impl<B: Backend> SimpleOptimizer<B> for RMSProp<B> {
    type State<const D: usize> = RMSPropState<B, D>;

    fn step<const D: usize>(
        &self,
        lr: LearningRate,
        tensor: Tensor<B, D>,
        mut grad: Tensor<B, D>,
        state: Option<Self::State<D>>,
    ) -> (Tensor<B, D>, Option<Self::State<D>>) {
        // fetch state for params
        let mut state_square_avg = None;
        let mut state_centered = None;
        let mut state_momentum = None;
        if let Some(state) = state {
            state_square_avg = Some(state.square_avg);
            state_centered = Some(state.centered);
            state_momentum = state.momentum;
        }

        // weight_decay transform
        if let Some(weight_decay) = &self.weight_decay {
            grad = weight_decay.transform_temp_fix(grad, tensor.clone());
        }

        // square_avg transform
        let (grad, state_square_avg) =
            SquareAvgState::transform(self.alpha, grad, state_square_avg);

        // centered transform
        let (grad, state_square_avg, state_centered) = CenteredState::transform(
            self.alpha,
            self.centered,
            grad,
            state_square_avg,
            state_centered,
        );

        // momentum transform
        let (grad, state_centered, state_momentum) =
            self.momentum
                .transform(grad, state_centered, state_momentum);

        // transition state
        let state = RMSPropState::new(state_square_avg, state_centered, state_momentum);

        // tensor param transform
        let delta = grad.mul_scalar(lr);
        (tensor - delta, Some(state))
    }

    fn to_device<const D: usize>(
        mut state: Self::State<D>,
        device: &<B as Backend>::Device,
    ) -> Self::State<D> {
        state.square_avg = state.square_avg.to_device(device);
        state.centered = state.centered.to_device(device);
        state.momentum = state.momentum.map(|momentum| momentum.to_device(device));
        state
    }
}

/// State of [RMSProp](RMSProp)
#[derive(Record, Clone, new)]
pub struct RMSPropState<B: Backend, const D: usize> {
    square_avg: SquareAvgState<B, D>,
    centered: CenteredState<B, D>,
    momentum: Option<RMSPropMomentumState<B, D>>,
}

/// [SquareAvgState](SquareAvgState) is to store and pass optimizer step params.
#[derive(Record, Clone, new)]
pub struct SquareAvgState<B: Backend, const D: usize> {
    square_avg: Tensor<B, D>,
}

impl<B: Backend, const D: usize> SquareAvgState<B, D> {
    /// transform [SquareAvgState] to the next step
    fn transform(alpha: f32, grad: Tensor<B, D>, state: Option<Self>) -> (Tensor<B, D>, Self) {
        match state {
            Some(state) => {
                let square_avg = state
                    .square_avg
                    .clone()
                    .mul_scalar(alpha)
                    .add(grad.clone().powf(2.).mul_scalar(1. - alpha));
                (grad, Self { square_avg })
            }
            _ => {
                let square_avg = grad.clone().powf(2.).mul_scalar(1. - alpha);
                (grad, Self { square_avg })
            }
        }
    }

    /// Moves the state to a device.
    ///
    /// # Arguments
    ///
    /// * `device` - Device to move the state to.
    ///
    /// # Returns
    ///
    /// * `self` - Moved state.
    pub fn to_device(mut self, device: &B::Device) -> Self {
        self.square_avg = self.square_avg.to_device(device);
        self
    }
}

/// [CenteredState](CenteredState) is to store and pass optimizer step params.
#[derive(Record, Clone, new)]
pub struct CenteredState<B: Backend, const D: usize> {
    grad_avg: Option<Tensor<B, D>>,
    avg: Tensor<B, D>,
}

impl<B: Backend, const D: usize> CenteredState<B, D> {
    /// transform [CenteredState] to the next step
    fn transform(
        alpha: f32,
        centered: bool,
        grad: Tensor<B, D>,
        square_avg_state: SquareAvgState<B, D>,
        centered_state: Option<Self>,
    ) -> (Tensor<B, D>, SquareAvgState<B, D>, Self) {
        if centered {
            let grad_avg_constant = grad.clone().mul_scalar(1. - alpha);
            let grad_avg = match centered_state {
                Some(state) => state
                    .grad_avg
                    .map_or(grad_avg_constant.clone(), move |grad_avg| {
                        grad_avg.clone().mul_scalar(alpha).add(grad_avg_constant)
                    }),
                _ => grad_avg_constant,
            };
            let avg = square_avg_state
                .square_avg
                .clone()
                .sub(grad_avg.clone().powf(2.));

            (
                grad,
                square_avg_state,
                Self {
                    grad_avg: Some(grad_avg),
                    avg,
                },
            )
        } else {
            (
                grad,
                square_avg_state.clone(),
                Self {
                    grad_avg: None,
                    avg: square_avg_state.square_avg,
                },
            )
        }
    }

    /// Moves the state to a device.
    ///
    /// # Arguments
    ///
    /// * `device` - Device to move the state to.
    ///
    /// # Returns
    ///
    /// * `self` - Moved state.
    pub fn to_device(mut self, device: &B::Device) -> Self {
        self.grad_avg = self.grad_avg.map(|grad_avg| grad_avg.to_device(device));
        self.avg = self.avg.to_device(device);
        self
    }
}

/// [RMSPropMomentum](RMSPropMomentum) is to store config status for optimizer.
/// (, which is stored in [optimizer](RMSProp) itself and not passed in during `step()` calculation)
pub struct RMSPropMomentum {
    momentum: f32,
    epsilon: f32,
}

impl RMSPropMomentum {
    /// transform [grad](Tensor) and [RMSPropMomentumState] to the next step
    fn transform<B: Backend, const D: usize>(
        &self,
        grad: Tensor<B, D>,
        centered_state: CenteredState<B, D>,
        momentum_state: Option<RMSPropMomentumState<B, D>>,
    ) -> (
        Tensor<B, D>,
        CenteredState<B, D>,
        Option<RMSPropMomentumState<B, D>>,
    ) {
        let grad = grad
            .clone()
            .div(centered_state.avg.clone().sqrt().add_scalar(self.epsilon));

        if self.momentum > 0. {
            let buf = match momentum_state {
                Some(state) => state
                    .buf
                    .clone()
                    .mul_scalar(self.momentum)
                    .add(grad.clone()),
                _ => grad.clone(),
            };
            (
                buf.clone(),
                centered_state,
                Some(RMSPropMomentumState { buf }),
            )
        } else {
            (grad.clone(), centered_state, None)
        }
    }
}

/// [RMSPropMomentumState](RMSPropMomentumState) is to store and pass optimizer step params.
#[derive(Record, Clone, new)]
pub struct RMSPropMomentumState<B: Backend, const D: usize> {
    buf: Tensor<B, D>,
}

impl<B: Backend, const D: usize> RMSPropMomentumState<B, D> {
    /// Moves the state to a device.
    ///
    /// # Arguments
    ///
    /// * `device` - Device to move the state to.
    ///
    /// # Returns
    ///
    /// * `self` - Moved state.
    pub fn to_device(mut self, device: &B::Device) -> Self {
        self.buf = self.buf.to_device(device);
        self
    }
}

#[cfg(test)]
mod tests {
    use burn_tensor::Shape;

    use super::*;
    use crate::module::{Module, Param};
    use crate::optim::{GradientsParams, Optimizer};
    use crate::record::{BinFileRecorder, FullPrecisionSettings, Recorder};
    use crate::tensor::{Data, Distribution, Tensor};
    use crate::{nn, TestADBackend, TestBackend};
    use tempfile::TempDir;

    const LEARNING_RATE: LearningRate = 0.01;
    const ASSERT_PRECISION: usize = 6;

    #[test]
    fn test_rmsprop_optimizer_save_load_state() {
        let linear = nn::LinearConfig::new(6, 6).init();
        let x = Tensor::<TestADBackend, 2>::random([2, 6], Distribution::Default);
        let mut optimizer = create_rmsprop();
        let grads = linear.forward(x).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let _linear = optimizer.step(LEARNING_RATE, linear, grads);
        let temp_dir = TempDir::new().unwrap();
        BinFileRecorder::<FullPrecisionSettings>::default()
            .record(optimizer.to_record(), temp_dir.path().join("test_optim"))
            .unwrap();

        let state_optim_before = optimizer.to_record();
        let state_optim_before_copy = optimizer.to_record();
        let optimizer = create_rmsprop();
        let optimizer = optimizer.load_record(state_optim_before_copy);
        let state_optim_after = optimizer.to_record();

        assert_eq!(state_optim_before.len(), state_optim_after.len());
    }

    /// used for test differences and debug
    #[test]
    fn test_rmsprop_optimizer_with_numbers_basic() {
        let linear = given_linear_layer(
            Data::from([
                [1., 1., 1., 1., 1., 1.],
                [1., 1., 1., 1., 1., 1.],
                [1., 1., 1., 1., 1., 1.],
                [1., 1., 1., 1., 1., 1.],
                [1., 1., 1., 1., 1., 1.],
                [1., 1., 1., 1., 1., 1.],
            ]),
            Data::from([0.5, 0.5, 0.5, 0.5, 0.5, 0.5]),
        );
        let x_1 = Tensor::from_floats([
            [0.6294, 0.0940, 0.8176, 0.8824, 0.5228, 0.4310],
            [0.7152, 0.9559, 0.7893, 0.5684, 0.5939, 0.8883],
        ])
        .require_grad();
        let x_2 = Tensor::from_floats([
            [0.8491, 0.2108, 0.8939, 0.4433, 0.5527, 0.2528],
            [0.3270, 0.0412, 0.5538, 0.9605, 0.3195, 0.9085],
        ])
        .require_grad();

        let mut optimizer = RMSPropConfig::new()
            .with_alpha(0.99)
            .with_epsilon(1e-8)
            .with_weight_decay(WeightDecayConfig::new(0.05).into())
            .with_momentum(0.9)
            .with_centered(false)
            .init();

        // println!("linear is {:?}", linear);
        let grads = linear.forward(x_1).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        // println!("linear is {:?}", linear);
        let grads = linear.forward(x_2).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        // println!("linear is {:?}", linear);
        let state_updated = linear.into_record();

        let (weight_updated, bias_updated) = (
            state_updated.weight.to_data(),
            state_updated.bias.unwrap().to_data(),
        );

        // println!("\nweight_updated\n{:?}", weight_updated);
        // println!("\nbias_updated\n{:?}", bias_updated);

        let weights_expected = Data::from([
            [0.743937, 0.743937, 0.743937, 0.743937, 0.743937, 0.743937],
            [0.783809, 0.783809, 0.783809, 0.783809, 0.783809, 0.783809],
            [0.742881, 0.742881, 0.742881, 0.742881, 0.742881, 0.742881],
            [0.740366, 0.740366, 0.740366, 0.740366, 0.740366, 0.740366],
            [0.748005, 0.748005, 0.748005, 0.748005, 0.748005, 0.748005],
            [0.743710, 0.743710, 0.743710, 0.743710, 0.743710, 0.743710],
        ]);
        let bias_expected =
            Data::from([0.239199, 0.239199, 0.239199, 0.239199, 0.239199, 0.239199]);

        bias_updated.assert_approx_eq(&bias_expected, ASSERT_PRECISION);
        weight_updated.assert_approx_eq(&weights_expected, ASSERT_PRECISION);
    }

    #[test]
    fn test_rmsprop_optimizer_with_numbers() {
        let linear = given_linear_layer(
            Data::from([
                [-0.3206, 0.1374, 0.4043, 0.3200, 0.0859, 0.0671],
                [0.0777, -0.0185, -0.3667, 0.2550, 0.1955, -0.2922],
                [-0.0190, 0.0346, -0.2962, 0.2484, -0.2780, 0.3130],
                [-0.2980, -0.2214, -0.3715, -0.2981, -0.0761, 0.1626],
                [0.3300, -0.2182, 0.3717, -0.1729, 0.3796, -0.0304],
                [-0.0159, -0.0120, 0.1258, 0.1921, 0.0293, 0.3833],
            ]),
            Data::from([-0.3905, 0.0884, -0.0970, 0.1176, 0.1366, 0.0130]),
        );
        let x_1 = Tensor::from_floats([
            [0.6294, 0.0940, 0.8176, 0.8824, 0.5228, 0.4310],
            [0.7152, 0.9559, 0.7893, 0.5684, 0.5939, 0.8883],
        ])
        .require_grad();
        let x_2 = Tensor::from_floats([
            [0.8491, 0.2108, 0.8939, 0.4433, 0.5527, 0.2528],
            [0.3270, 0.0412, 0.5538, 0.9605, 0.3195, 0.9085],
        ])
        .require_grad();

        let mut optimizer = RMSPropConfig::new()
            .with_alpha(0.99)
            .with_epsilon(1e-8)
            .with_weight_decay(WeightDecayConfig::new(0.05).into())
            .with_momentum(0.9)
            .with_centered(false)
            .init();

        let grads = linear.forward(x_1).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        let grads = linear.forward(x_2).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        let state_updated = linear.into_record();
        let weights_expected = Data::from([
            [
                -0.576399, -0.118494, 0.148353, 0.064070, -0.169983, -0.188779,
            ],
            [
                -0.135571, -0.231448, -0.578445, 0.041143, -0.018162, -0.504207,
            ],
            [
                -0.275990, -0.222397, -0.553153, -0.008625, -0.534956, 0.055967,
            ],
            [
                -0.557575, -0.480979, -0.631072, -0.557675, -0.335686, -0.096997,
            ],
            [
                0.078313, -0.469618, 0.119993, -0.424341, 0.127890, -0.281912,
            ],
            [
                -0.271996, -0.268097, -0.130324, -0.064037, -0.226805, 0.127126,
            ],
        ]);
        let bias_expected = Data::from([
            -0.651299, -0.172400, -0.357800, -0.143200, -0.124200, -0.247800,
        ]);

        let (weight_updated, bias_updated) = (
            state_updated.weight.to_data(),
            state_updated.bias.unwrap().to_data(),
        );

        // println!("\nweight_updated\n{:?}", weight_updated);
        // println!("\nbias_updated\n{:?}", bias_updated);

        bias_updated.assert_approx_eq(&bias_expected, ASSERT_PRECISION);
        weight_updated.assert_approx_eq(&weights_expected, ASSERT_PRECISION);
    }

    fn given_linear_layer(weight: Data<f32, 2>, bias: Data<f32, 1>) -> nn::Linear<TestADBackend> {
        let record = nn::LinearRecord {
            weight: Param::from(Tensor::from_data(weight)),
            bias: Some(Param::from(Tensor::from_data(bias))),
        };

        nn::LinearConfig::new(6, 6).init_with(record)
    }

    #[allow(dead_code)]
    fn create_random_tensor() -> Tensor<TestADBackend, 2> {
        Tensor::<TestADBackend, 2>::random(Shape::new([2, 20]), Distribution::Default)
    }

    fn create_rmsprop(
    ) -> OptimizerAdaptor<RMSProp<TestBackend>, nn::Linear<TestADBackend>, TestADBackend> {
        RMSPropConfig {
            alpha: 0.99,
            epsilon: 1e-9,
            centered: false,
            weight_decay: Some(WeightDecayConfig { penalty: 0.05 }),
            momentum: 0.9,
            grad_clipping: None,
            ..RMSPropConfig::new()
        }
        .init()
    }
}
