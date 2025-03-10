/// Include generated models in the `model` directory in the target directory.
macro_rules! include_models {
    ($($model:ident),*) => {
        $(
            pub mod $model {
                include!(concat!(env!("OUT_DIR"), concat!("/model/", stringify!($model), ".rs")));
            }
        )*
    };
}

// ATTENTION: Modify this macro to include all models in the `model` directory.
include_models!(
    add,
    avg_pool2d,
    concat,
    conv1d,
    conv2d,
    div,
    dropout_opset16,
    dropout_opset7,
    flatten,
    global_avr_pool,
    log_softmax,
    maxpool2d,
    mul,
    reshape,
    softmax,
    sub
);

#[cfg(test)]
mod tests {
    use super::*;

    use burn::tensor::{Data, Shape, Tensor};

    use float_cmp::ApproxEq;

    type Backend = burn_ndarray::NdArrayBackend<f32>;

    #[test]
    fn add_scalar_to_tensor_and_tensor_to_tensor() {
        // Initialize the model with weights (loaded from the exported file)
        let model: add::Model<Backend> = add::Model::default();

        // Run the model
        let input = Tensor::<Backend, 4>::from_floats([[[[1., 2., 3., 4.]]]]);
        let scalar = 2f64;
        let output = model.forward(input, scalar);
        let expected = Data::from([[[[9., 10., 11., 12.]]]]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn sub_scalar_from_tensor_and_tensor_from_tensor() {
        // Initialize the model with weights (loaded from the exported file)
        let model: sub::Model<Backend> = sub::Model::default();

        // Run the model
        let input = Tensor::<Backend, 4>::from_floats([[[[1., 2., 3., 4.]]]]);
        let scalar = 3.0f64;
        let output = model.forward(input, scalar);
        let expected = Data::from([[[[6., 7., 8., 9.]]]]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn mul_scalar_with_tensor_and_tensor_with_tensor() {
        // Initialize the model with weights (loaded from the exported file)
        let model: mul::Model<Backend> = mul::Model::default();

        // Run the model
        let input = Tensor::<Backend, 4>::from_floats([[[[1., 2., 3., 4.]]]]);
        let scalar = 6.0f64;
        let output = model.forward(input, scalar);
        let expected = Data::from([[[[126., 252., 378., 504.]]]]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn div_tensor_by_scalar_and_tensor_by_tensor() {
        // Initialize the model without weights (because the exported file does not contain them)
        let model: div::Model<Backend> = div::Model::new();

        // Run the model
        let input = Tensor::<Backend, 4>::from_floats([[[[3., 6., 6., 9.]]]]);
        let scalar1 = 9.0f64;
        let scalar2 = 3.0f64;
        let output = model.forward(input, scalar1, scalar2);
        let expected = Data::from([[[[1., 2., 2., 3.]]]]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn concat_tensors() {
        // Initialize the model
        let model: concat::Model<Backend> = concat::Model::new();

        // Run the model
        let input = Tensor::<Backend, 4>::zeros([1, 2, 3, 5]);

        let output = model.forward(input);

        let expected = Shape::from([1, 18, 3, 5]);

        assert_eq!(output.shape(), expected);
    }

    #[test]
    fn conv1d() {
        // Initialize the model with weights (loaded from the exported file)
        let model: conv1d::Model<Backend> = conv1d::Model::default();

        // Run the model with ones as input for easier testing
        let input = Tensor::<Backend, 3>::ones([1, 4, 5]);

        let output = model.forward(input);

        // test the output shape
        let expected_shape: Shape<3> = Shape::from([1, 2, 11]);
        assert_eq!(output.shape(), expected_shape);

        // We are using the sum of the output tensor to test the correctness of the conv1d node
        // because the output tensor is too large to compare with the expected tensor.
        let output_sum = output.sum().into_scalar();
        let expected_sum = -1.722_538; // from pytorch
        assert!(expected_sum.approx_eq(output_sum, (1.0e-4, 2)));
    }

    #[test]
    fn conv2d() {
        // Initialize the model with weights (loaded from the exported file)
        let model: conv2d::Model<Backend> = conv2d::Model::default();

        // Run the model with ones as input for easier testing
        let input = Tensor::<Backend, 4>::ones([2, 4, 10, 15]);

        let output = model.forward(input);

        let expected_shape = Shape::from([2, 6, 6, 15]);
        assert_eq!(output.shape(), expected_shape);

        // We are using the sum of the output tensor to test the correctness of the conv2d node
        // because the output tensor is too large to compare with the expected tensor.
        let output_sum = output.sum().into_scalar();

        let expected_sum = -113.869_99; // from pytorch

        assert!(expected_sum.approx_eq(output_sum, (1.0e-4, 2)));
    }

    #[test]
    fn dropout_opset16() {
        let model: dropout_opset16::Model<Backend> = dropout_opset16::Model::default();

        // Run the model with ones as input for easier testing
        let input = Tensor::<Backend, 4>::ones([2, 4, 10, 15]);

        let output = model.forward(input);

        let expected_shape = Shape::from([2, 4, 10, 15]);
        assert_eq!(output.shape(), expected_shape);

        let output_sum = output.sum().into_scalar();

        let expected_sum = 1200.0; // from pytorch

        assert!(expected_sum.approx_eq(output_sum, (1.0e-4, 2)));
    }

    #[test]
    fn dropout_opset7() {
        let model: dropout_opset7::Model<Backend> = dropout_opset7::Model::default();

        // Run the model with ones as input for easier testing
        let input = Tensor::<Backend, 4>::ones([2, 4, 10, 15]);

        let output = model.forward(input);

        let expected_shape = Shape::from([2, 4, 10, 15]);
        assert_eq!(output.shape(), expected_shape);

        let output_sum = output.sum().into_scalar();

        let expected_sum = 1200.0; // from pytorch

        assert!(expected_sum.approx_eq(output_sum, (1.0e-4, 2)));
    }

    #[test]
    fn globalavrpool_1d_2d() {
        // The model contains 1d and 2d global average pooling nodes
        let model: global_avr_pool::Model<Backend> = global_avr_pool::Model::default();

        // Run the model with ones as input for easier testing
        let input_1d = Tensor::<Backend, 3>::ones([2, 4, 10]);
        let input_2d = Tensor::<Backend, 4>::ones([3, 10, 3, 15]);

        let (output_1d, output_2d) = model.forward(input_1d, input_2d);

        let expected_shape_1d = Shape::from([2, 4, 1]);
        let expected_shape_2d = Shape::from([3, 10, 1, 1]);
        assert_eq!(output_1d.shape(), expected_shape_1d);
        assert_eq!(output_2d.shape(), expected_shape_2d);

        let output_sum_1d = output_1d.sum().into_scalar();
        let output_sum_2d = output_2d.sum().into_scalar();

        let expected_sum_1d = 8.0; // from pytorch
        let expected_sum_2d = 30.0; // from pytorch

        assert!(expected_sum_1d.approx_eq(output_sum_1d, (1.0e-4, 2)));
        assert!(expected_sum_2d.approx_eq(output_sum_2d, (1.0e-4, 2)));
    }

    #[test]
    fn softmax() {
        // Initialize the model without weights (because the exported file does not contain them)
        let model: softmax::Model<Backend> = softmax::Model::new();

        // Run the model
        let input = Tensor::<Backend, 2>::from_floats([
            [0.33669037, 0.128_809_4, 0.23446237],
            [0.23033303, -1.122_856_4, -0.18632829],
        ]);
        let output = model.forward(input);
        let expected = Data::from([
            [0.36830685, 0.29917702, 0.33251613],
            [0.521_469_2, 0.13475533, 0.343_775_5],
        ]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn log_softmax() {
        // Initialize the model without weights (because the exported file does not contain them)
        let model: log_softmax::Model<Backend> = log_softmax::Model::new();

        // Run the model
        let input = Tensor::<Backend, 2>::from_floats([
            [0.33669037, 0.128_809_4, 0.23446237],
            [0.23033303, -1.122_856_4, -0.18632829],
        ]);
        let output = model.forward(input);
        let expected = Data::from([
            [-0.998_838_9, -1.206_719_9, -1.101_067],
            [-0.651_105_1, -2.004_294_6, -1.067_766_4],
        ]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn maxpool2d() {
        // Initialize the model without weights (because the exported file does not contain them)
        let model: maxpool2d::Model<Backend> = maxpool2d::Model::new();

        // Run the model
        let input = Tensor::<Backend, 4>::from_floats([[[
            [1.927, 1.487, 0.901, -2.106, 0.678],
            [-1.235, -0.043, -1.605, -0.752, -0.687],
            [-0.493, 0.241, -1.111, 0.092, -2.317],
            [-0.217, -1.385, -0.396, 0.803, -0.622],
            [-0.592, -0.063, -0.829, 0.331, -1.558],
        ]]]);
        let output = model.forward(input);
        let expected = Data::from([[[
            [1.927, 1.927, 1.487, 0.901, 0.678, 0.678],
            [1.927, 1.927, 1.487, 0.901, 0.803, 0.678],
            [-0.217, 0.241, 0.241, 0.803, 0.803, -0.622],
        ]]]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn avg_pool2d() {
        // Initialize the model without weights (because the exported file does not contain them)
        let model: avg_pool2d::Model<Backend> = avg_pool2d::Model::new();

        // Run the model
        let input = Tensor::<Backend, 4>::from_floats([[[
            [-0.077, 0.360, -0.782, 0.072, 0.665],
            [-0.287, 1.621, -1.597, -0.052, 0.611],
            [0.760, -0.034, -0.345, 0.494, -0.078],
            [-1.805, -0.476, 0.205, 0.338, 1.353],
            [0.374, 0.013, 0.774, -0.109, -0.271],
        ]]]);
        let output = model.forward(input);
        let expected = Data::from([[[[0.008, -0.131, -0.208, 0.425]]]]);

        output.to_data().assert_approx_eq(&expected, 3);
    }

    #[test]
    fn reshape() {
        // Initialize the model without weights (because the exported file does not contain them)
        let model: reshape::Model<Backend> = reshape::Model::new();

        // Run the model
        let input = Tensor::<Backend, 1>::from_floats([0., 1., 2., 3.]);
        let output = model.forward(input);
        let expected = Data::from([[0., 1., 2., 3.]]);

        assert_eq!(output.to_data(), expected);
    }

    #[test]
    fn flatten() {
        // Initialize the model without weights (because the exported file does not contain them)
        let model: flatten::Model<Backend> = flatten::Model::new();

        // Run the model
        let input = Tensor::<Backend, 3>::ones([1, 5, 15]);
        let output = model.forward(input);

        let expected_shape = Shape::from([1, 75]);
        assert_eq!(expected_shape, output.shape());
    }
}
