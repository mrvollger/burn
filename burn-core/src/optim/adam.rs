use crate::{
    self as burn, grad_clipping::GradientClippingConfig, module::ADModule, record::Record,
    LearningRate,
};

use super::{
    decay::{WeightDecay, WeightDecayConfig, WeightDecayState},
    Optimizer, SimpleOptimizer,
};
use crate::config::Config;
use crate::optim::adaptor::OptimizerAdaptor;
use crate::tensor::{backend::ADBackend, Tensor};
use burn_tensor::{backend::Backend, ElementConversion};

/// Adam configuration.
#[derive(Config)]
pub struct AdamConfig {
    /// Parameter for Adam.
    #[config(default = 0.9)]
    beta_1: f32,
    /// Parameter for Adam.
    #[config(default = 0.999)]
    beta_2: f32,
    /// A value required for numerical stability.
    #[config(default = 1e-5)]
    epsilon: f32,
    /// [Weight decay](WeightDecayConfig) config.
    weight_decay: Option<WeightDecayConfig>,
    /// [Gradient Clipping](GradientClippingConfig) config.
    grad_clipping: Option<GradientClippingConfig>,
}

/// Adam optimizer as described in the paper [Adam: A Method for Stochastic Optimization](https://arxiv.org/pdf/1412.6980.pdf).
pub struct Adam<B: Backend> {
    momentum: AdaptiveMomentum,
    weight_decay: Option<WeightDecay<B>>,
}

/// Adam state.
#[derive(Record, Clone, new)]
pub struct AdamState<B: Backend, const D: usize> {
    weight_decay: Option<WeightDecayState<B, D>>,
    momentum: AdaptiveMomentumState<B, D>,
}

impl<B: Backend> SimpleOptimizer<B> for Adam<B> {
    type State<const D: usize> = AdamState<B, D>;

    fn step<const D: usize>(
        &self,
        lr: LearningRate,
        tensor: Tensor<B, D>,
        mut grad: Tensor<B, D>,
        state: Option<Self::State<D>>,
    ) -> (Tensor<B, D>, Option<Self::State<D>>) {
        let mut state_weight_decay = None;
        let mut state_momentum = None;

        if let Some(state) = state {
            state_weight_decay = state.weight_decay;
            state_momentum = Some(state.momentum);
        }

        if let Some(weight_decay) = &self.weight_decay {
            let (grad_out, state) = weight_decay.transform(grad, state_weight_decay);
            state_weight_decay = Some(state);
            grad = grad_out;
        }

        let (grad, state_momentum) = self.momentum.transform(grad, state_momentum);

        let state = AdamState::new(state_weight_decay, state_momentum);
        let delta = grad.mul_scalar(lr);

        (tensor - delta, Some(state))
    }

    fn to_device<const D: usize>(
        mut state: Self::State<D>,
        device: &<B as Backend>::Device,
    ) -> Self::State<D> {
        state.weight_decay = state.weight_decay.map(|state| state.to_device(device));
        state.momentum = state.momentum.to_device(device);
        state
    }
}

impl AdamConfig {
    /// Initialize Adam optimizer.
    ///
    /// # Returns
    ///
    /// Returns an optimizer that can be used to optimize a module.
    pub fn init<B: ADBackend, M: ADModule<B>>(&self) -> impl Optimizer<M, B> {
        let optim = Adam {
            momentum: AdaptiveMomentum {
                beta_1: self.beta_1,
                beta_2: self.beta_2,
                epsilon: self.epsilon,
            },
            weight_decay: self.weight_decay.as_ref().map(WeightDecay::new),
        };

        let mut optim = OptimizerAdaptor::from(optim);
        if let Some(config) = &self.grad_clipping {
            optim = optim.with_grad_clipping(config.init());
        }
        optim
    }
}

/// Adaptive momentum state.
#[derive(Record, new, Clone)]
pub struct AdaptiveMomentumState<B: Backend, const D: usize> {
    time: usize,
    moment_1: Tensor<B, D>,
    moment_2: Tensor<B, D>,
}

struct AdaptiveMomentum {
    beta_1: f32,
    beta_2: f32,
    epsilon: f32,
}

impl AdaptiveMomentum {
    pub fn transform<B: Backend, const D: usize>(
        &self,
        grad: Tensor<B, D>,
        momentum_state: Option<AdaptiveMomentumState<B, D>>,
    ) -> (Tensor<B, D>, AdaptiveMomentumState<B, D>) {
        let state = if let Some(mut state) = momentum_state {
            let factor = 1.0 - self.beta_1;
            state.moment_1 = state
                .moment_1
                .mul_scalar(self.beta_1)
                .add(grad.clone().mul_scalar(factor));

            let factor = 1.0 - self.beta_2;
            state.moment_2 = state
                .moment_2
                .mul_scalar(self.beta_2)
                .add(grad.powf(2.0).mul_scalar(factor));

            state.time += 1;

            state
        } else {
            let factor = 1.0 - self.beta_1;
            let moment_1 = grad.clone().mul_scalar(factor);

            let factor = 1.0 - self.beta_2;
            let moment_2 = grad.powf(2.0).mul_scalar(factor);

            AdaptiveMomentumState::new(1, moment_1, moment_2)
        };

        let time = (state.time as i32).elem();
        let moment_1_corrected = state
            .moment_1
            .clone()
            .div_scalar(1f32 - self.beta_1.powi(time));
        let moment_2_corrected = state
            .moment_2
            .clone()
            .div_scalar(1f32 - self.beta_2.powi(time));

        let grad = moment_1_corrected.div(moment_2_corrected.sqrt().add_scalar(self.epsilon));

        (grad, state)
    }
}

impl<B: Backend, const D: usize> AdaptiveMomentumState<B, D> {
    /// Move state to device.
    ///
    /// # Arguments
    ///
    /// * `device` - Device to move state to.
    ///
    /// # Returns
    ///
    /// Returns state moved to device.
    pub fn to_device(mut self, device: &B::Device) -> Self {
        self.moment_1 = self.moment_1.to_device(device);
        self.moment_2 = self.moment_2.to_device(device);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{Module, Param};
    use crate::optim::{GradientsParams, Optimizer};
    use crate::record::{BinFileRecorder, FullPrecisionSettings, Recorder};
    use crate::tensor::{Data, Distribution, Tensor};
    use crate::{nn, TestADBackend, TestBackend};

    const LEARNING_RATE: LearningRate = 0.01;

    #[test]
    fn test_adam_optimizer_save_load_state() {
        let linear = nn::LinearConfig::new(6, 6).init();
        let x = Tensor::<TestADBackend, 2>::random([2, 6], Distribution::Default);
        let mut optimizer = create_adam();
        let grads = linear.forward(x).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let _linear = optimizer.step(LEARNING_RATE, linear, grads);
        BinFileRecorder::<FullPrecisionSettings>::default()
            .record(optimizer.to_record(), "/tmp/test_optim".into())
            .unwrap();

        let state_optim_before = optimizer.to_record();
        let state_optim_before_copy = optimizer.to_record();
        let optimizer = create_adam();
        let optimizer = optimizer.load_record(state_optim_before_copy);
        let state_optim_after = optimizer.to_record();

        assert_eq!(state_optim_before.len(), state_optim_after.len());
    }
    const ASSERT_PRECISION: usize = 2;

    #[test]
    fn test_adam_optimizer_with_numbers() {
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

        let mut optimizer = AdamConfig::new()
            .with_epsilon(1e-8)
            .with_beta_1(0.9)
            .with_beta_2(0.999)
            .with_weight_decay(Some(WeightDecayConfig::new(0.5)))
            .init();

        let grads = linear.forward(x_1).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        let grads = linear.forward(x_2).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        let state_updated = linear.into_record();
        let weights_expected = Data::from([
            [-0.340528, 0.118929, 0.384336, 0.300010, 0.066034, 0.047154],
            [
                0.057757, -0.036690, -0.386649, 0.235010, 0.175624, -0.312133,
            ],
            [
                -0.038940, 0.016306, -0.316151, 0.228410, -0.297819, 0.293047,
            ],
            [
                -0.317929, -0.239100, -0.391449, -0.318087, -0.095948, 0.142651,
            ],
            [
                0.310050, -0.235909, 0.351736, -0.192888, 0.359710, -0.050343,
            ],
            [-0.035840, -0.030203, 0.105840, 0.172110, 0.009440, 0.363346],
        ]);
        let bias_expected = Data::from([
            -0.410499, 0.068401, -0.116999, 0.097601, 0.116601, -0.006999,
        ]);

        let (weight_updated, bias_updated) = (
            state_updated.weight.to_data(),
            state_updated.bias.unwrap().to_data(),
        );

        bias_updated.assert_approx_eq(&bias_expected, ASSERT_PRECISION);
        weight_updated.assert_approx_eq(&weights_expected, ASSERT_PRECISION);
    }

    #[test]
    fn test_adam_optimizer_no_nan() {
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

        let x = Tensor::from_floats([
            [0.8491, 0.2108, 0.8939, 0.4433, 0.5527, 0.2528],
            [0.3270, 0.0412, 0.5538, 0.9605, 0.3195, 0.9085],
        ])
        .require_grad();

        let mut optimizer = AdamConfig::new()
            .with_epsilon(1e-8)
            .with_beta_1(0.9)
            .with_beta_2(0.999)
            .with_weight_decay(Some(WeightDecayConfig::new(0.5)))
            .init();

        let grads = linear.forward(x.clone()).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        let grads = linear.forward(x).backward();
        let grads = GradientsParams::from_grads(grads, &linear);
        let linear = optimizer.step(LEARNING_RATE, linear, grads);

        let state_updated = linear.into_record();
        assert!(!state_updated.weight.to_data().value[0].is_nan());
    }

    fn given_linear_layer(weight: Data<f32, 2>, bias: Data<f32, 1>) -> nn::Linear<TestADBackend> {
        let record = nn::LinearRecord {
            weight: Param::from(Tensor::from_data(weight)),
            bias: Some(Param::from(Tensor::from_data(bias))),
        };

        nn::LinearConfig::new(6, 6).init_with(record)
    }

    fn create_adam() -> OptimizerAdaptor<Adam<TestBackend>, nn::Linear<TestADBackend>, TestADBackend>
    {
        let config = AdamConfig::new();
        Adam {
            momentum: AdaptiveMomentum {
                beta_1: config.beta_1,
                beta_2: config.beta_2,
                epsilon: config.epsilon,
            },
            weight_decay: config.weight_decay.as_ref().map(WeightDecay::new),
        }
        .into()
    }
}
