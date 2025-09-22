//! Bivariate polynomial compatible with arkwork's trait and backend

use ark_ff::{Field, Zero};
use ark_poly::{univariate, DenseUVPolynomial, Polynomial};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::{
    fmt,
    iter::successors,
    ops::{Add, AddAssign, Neg, Sub, SubAssign},
    rand::Rng,
};
use p3_maybe_rayon::prelude::{IndexedParallelIterator, ParallelIterator};

// NOTE: I have avoid implmenting `trait DenseMVPolynomial`, because the
// knowledge of "dense+bivariate" obviates the need to define `BVTerm` and
// `num_vars()` etc. I will consider adding them if necessary down the road.
// TODO: avoid panic and define custom error type

/// A dense bivariate polynomial
///
/// # Representation
///
/// Dense polynomial assumes coefficients for all possible monomials.
/// General Matrix Form (Row-wise Storage):
///
/// [  X^0 * Y^0   X^0 * Y^1   X^0 * Y^2   ...   X^0 * Y^d_y  ]
/// [  X^1 * Y^0   X^1 * Y^1   X^1 * Y^2   ...   X^1 * Y^d_y  ]
/// [  X^2 * Y^0   X^2 * Y^1   X^2 * Y^2   ...   X^2 * Y^d_y  ]
/// [     ...          ...          ...    ...       ...      ]
/// [ X^d_x * Y^0  X^d_x * Y^1  X^d_x * Y^2  ...  X^d_x * Y^d_y ]
///
/// Matrix Dimensions: (d_x + 1) x (d_y + 1)
///
/// Row-wise iteration:
/// First fix the power of X (row), then iterate over powers of Y (columns).
///
/// Example: For d_x = 2, d_y = 2:
/// Matrix:
/// [ 1     1 * Y      1 * Y^2   ]
/// [ X     X * Y      X * Y^2   ]
/// [ X^2   X^2 * Y   X^2 * Y^2  ]
#[derive(Clone, PartialEq, Eq, Hash, CanonicalSerialize, CanonicalDeserialize)]
pub struct DensePolynomial<F: Field> {
    /// coefficients matrix for monomial terms:
    pub coeffs: Vec<Vec<F>>,
    /// degree in X
    pub deg_x: usize,
    /// degree in Y
    pub deg_y: usize,
}

impl<F: Field> Polynomial<F> for DensePolynomial<F> {
    type Point = (F, F);
    fn degree(&self) -> usize {
        self.deg_x * self.deg_y
    }
    // full evaluation
    fn evaluate(&self, (x, y): &Self::Point) -> F {
        self.coeffs
            .iter()
            .enumerate()
            .map(|(row_idx, row)| {
                row.iter()
                    .enumerate()
                    .map(|(col_idx, coeff)| {
                        *coeff * x.pow([row_idx as u64]) * y.pow([col_idx as u64])
                    })
                    .fold(F::ZERO, |acc, term| acc + term)
            })
            .fold(F::ZERO, |acc, row_sum| acc + row_sum)
    }
}

impl<F: Field> DensePolynomial<F> {
    /// constructor with check
    pub fn new(coeffs: Vec<Vec<F>>, deg_x: usize, deg_y: usize) -> Self {
        assert!(
            !coeffs.is_empty(),
            "empty coeffs, use ::zero() or ::default() for zero poly"
        );
        assert!(coeffs.len() > 0 && coeffs.len() == deg_x + 1);
        assert!(coeffs[0].len() > 0 && coeffs.iter().all(|row| row.len() == deg_y + 1));
        Self {
            coeffs,
            deg_x,
            deg_y,
        }
    }
    /// Similar to `Self::zero()` but reserve space for degree>0
    pub fn zero_with(deg_x: usize, deg_y: usize) -> Self {
        Self {
            coeffs: vec![vec![F::zero(); deg_y + 1]; deg_x + 1],
            deg_x,
            deg_y,
        }
    }

    /// generate a random bivariate poly with `d_x` degree in X, `d_y` degree in
    /// Y
    pub fn rand<R: Rng>(deg_x: usize, deg_y: usize, rng: &mut R) -> Self {
        let coeffs = (0..deg_x + 1)
            .map(|_| (0..deg_y + 1).map(|_| F::rand(rng)).collect())
            .collect();
        Self {
            coeffs,
            deg_x,
            deg_y,
        }
    }

    /// Partial-evaluate f(X, Y) at concrete x, returns univariate poly G(Y) =
    /// f(x, Y)
    pub fn partial_evaluate_at_x(&self, x: &F) -> univariate::DensePolynomial<F> {
        // first compute v = (1, x, x^2, ... , x^d_x),
        let x_pows: Vec<F> = successors(Some(F::ONE), |&prev| Some(prev * x))
            .take(self.deg_x + 1)
            .collect();

        // then dot product with each column of coefficient
        let coeffs = self
            .coeffs
            .iter()
            .enumerate()
            .map(|(row_idx, row)| {
                let x_pow = x_pows[row_idx];
                row.iter().map(|c| x_pow * c).collect::<Vec<F>>()
            })
            .reduce(|mut acc, powed_row| {
                acc.iter_mut()
                    .zip(powed_row.iter())
                    .for_each(|(acc, &val)| *acc += val);
                acc
            })
            .unwrap_or_else(|| vec![F::ZERO; self.deg_y + 1]);

        univariate::DensePolynomial::from_coefficients_vec(coeffs)
    }

    /// Partial-evaluate f(X, Y) at concrete y, returns univariate poly G(X) =
    /// f(X, y)
    pub fn partial_evaluate_at_y(&self, y: &F) -> univariate::DensePolynomial<F> {
        // first compute v = (1, y, y^2, ..., y^d_y),
        let y_pows: Vec<F> = successors(Some(F::ONE), |&prev| Some(prev * y))
            .take(self.deg_y + 1)
            .collect();

        // then dot product with each row of coefficient
        let coeffs: Vec<F> = self
            .coeffs
            .iter()
            .map(|row| {
                row.iter()
                    .zip(y_pows.iter())
                    .map(|(c, y_pow)| *c * y_pow)
                    .sum::<F>()
            })
            .collect();
        univariate::DensePolynomial::from_coefficients_vec(coeffs)
    }

    /// divide `self` with another univariate (in X or Y), returns both the
    /// quotient and remainder f(X, Y) = q(X, Y) * divisor + r(X, Y)
    /// divisor := g(X) if `in_x=true`; divisor := g(Y) if `in_x=false`.
    pub fn divide_with_q_and_r(
        &self,
        divisor: &univariate::DensePolynomial<F>,
        in_x: bool,
    ) -> Option<(DensePolynomial<F>, DensePolynomial<F>)> {
        if self.is_zero() {
            Some((Self::zero(), Self::zero()))
        } else if divisor.is_zero() {
            panic!("Dividing by zero poly")
        } else if in_x && self.deg_x < divisor.degree() {
            Some((Self::zero(), self.clone()))
        } else if !in_x && self.deg_y < divisor.degree() {
            Some((Self::zero(), self.clone()))
        } else if divisor.degree() == 0 {
            let div_inv = divisor.coeffs[0].inverse().unwrap();
            let mut quotient = self.clone();
            quotient
                .coeffs
                .iter_mut()
                .for_each(|row| row.iter_mut().for_each(|c| *c *= div_inv));
            quotient.update_degree();
            Some((quotient, Self::zero()))
        } else {
            if in_x {
                let mut quotient = Self::zero_with(self.deg_x - divisor.degree(), self.deg_y);
                let mut remainder = self.clone();
                let divisor_leading_inv = divisor.last().unwrap().inverse().unwrap();
                while !remainder.is_zero() && remainder.deg_x >= divisor.degree() {
                    let cur_q_coeff: Vec<F> = remainder
                        .coeffs
                        .last()
                        .unwrap()
                        .iter()
                        .map(|&c| c * &divisor_leading_inv)
                        .collect();
                    let cur_q_degree = remainder.deg_x - divisor.degree();
                    quotient.coeffs[cur_q_degree] = cur_q_coeff.clone();

                    divisor.iter().enumerate().for_each(|(i, div_coeff)| {
                        remainder.coeffs[cur_q_degree + i]
                            .iter_mut()
                            .zip(cur_q_coeff.iter())
                            .for_each(|(c, q_c)| *c -= *q_c * div_coeff)
                    });
                    if let Some(row) = remainder.coeffs.last() {
                        assert!(row.iter().all(|c| c.is_zero()));
                        remainder.coeffs.pop();
                        remainder.deg_x -= 1;
                    }
                }
                quotient.update_degree();
                remainder.update_degree();

                assert!(
                    remainder.deg_x < divisor.degree()
                        && quotient.deg_x + divisor.degree() == self.deg_x
                );
                Some((quotient, remainder))
            } else {
                let mut quotient = Self::zero_with(self.deg_x, self.deg_y - divisor.degree());
                let mut remainder = self.clone();
                let divisor_leading_inv = divisor.last().unwrap().inverse().unwrap();
                while !remainder.is_zero() && remainder.deg_y >= divisor.degree() {
                    let cur_q_coeff: Vec<F> = remainder
                        .coeffs
                        .iter()
                        .map(|row| {
                            let c = row.last().unwrap();
                            *c * &divisor_leading_inv
                        })
                        .collect();
                    let cur_q_degree = remainder.deg_y - divisor.degree();
                    quotient
                        .coeffs
                        .iter_mut()
                        .enumerate()
                        .for_each(|(idx, row)| row[cur_q_degree] = cur_q_coeff[idx]);
                    divisor.iter().enumerate().for_each(|(i, div_coeff)| {
                        remainder
                            .coeffs
                            .iter_mut()
                            .enumerate()
                            .for_each(|(idx, row)| {
                                row[cur_q_degree + i] -= cur_q_coeff[idx] * div_coeff
                            })
                    });
                    if let Some(column) = remainder
                        .coeffs
                        .iter_mut()
                        .map(|row| row.last())
                        .collect::<Option<Vec<_>>>()
                    {
                        assert!(column.iter().all(|c| c.is_zero()));
                        remainder.coeffs.iter_mut().for_each(|row| {
                            row.pop();
                        });
                        remainder.deg_y -= 1;
                    }
                }
                quotient.update_degree();
                remainder.update_degree();
                assert!(
                    remainder.deg_y < divisor.degree()
                        && quotient.deg_y + divisor.degree() == self.deg_y
                );
                Some((quotient, remainder))
            }
        }
    }

    /// f'(X, Y) = f(X, Y) - g(X), if `in_x=true`
    /// f'(X, Y) = f(X, Y) - g(Y), if `in_x=false`
    pub fn sub_assign_uv_poly(&mut self, uv_poly: &univariate::DensePolynomial<F>, in_x: bool) {
        if uv_poly.is_zero() {
        } else {
            if in_x {
                assert!(
                    uv_poly.degree() <= self.deg_x,
                    "only support subtracting smaller deg univariate for now"
                );
                self.coeffs
                    .iter_mut()
                    .zip(uv_poly.coeffs.iter())
                    .for_each(|(row, sub_term)| row[0] -= *sub_term);
                self.update_degree();
            } else {
                assert!(
                    uv_poly.degree() <= self.deg_y,
                    "only support subtracting smaller deg univariate for now"
                );
                self.coeffs[0]
                    .iter_mut()
                    .zip(uv_poly.coeffs.iter())
                    .for_each(|(c, sub_term)| *c -= *sub_term);
            }
        }
    }

    // adjust/decrease degree in case there are leading zeros in X or Y
    fn update_degree(&mut self) {
        if self.is_zero() {
            *self = Self::zero();
            return;
        }
        // adjust deg_x
        while let Some(row) = self.coeffs.last() {
            if row.iter().all(|c| c.is_zero()) {
                self.coeffs.pop();
                self.deg_x -= 1;
            } else {
                break; // Stop when a non-zero row is encountered
            }
        }
        // adjust deg_y
        let mut cols = self.deg_y + 1;
        while cols > 0 {
            cols -= 1;
            // Check if the column is entirely zero
            if self.coeffs.iter().all(|row| row[cols].is_zero()) {
                // remove the trailing zero in all rows
                self.coeffs.iter_mut().for_each(|row| {
                    row.pop();
                });
                self.deg_y -= 1;
            } else {
                // Stop if we find a non-zero (trailing) column
                break;
            }
        }
        assert!(!self.coeffs.is_empty(), "should never be empty");
    }
}

// === Basic Operations for Bivarate Polynomial ====
// =================================================
// TODO: impl Mul, Div
impl<F: Field> Add for DensePolynomial<F> {
    type Output = DensePolynomial<F>;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        &self + &rhs
    }
}

impl<'a, 'b, F: Field> Add<&'a DensePolynomial<F>> for &'b DensePolynomial<F> {
    type Output = DensePolynomial<F>;
    #[inline]
    fn add(self, rhs: &'a DensePolynomial<F>) -> Self::Output {
        if rhs.is_zero() {
            self.clone()
        } else if self.is_zero() {
            rhs.clone()
        } else if self.deg_y == rhs.deg_y && self.deg_x == rhs.deg_x {
            // slightly more memory efficient than the next general case
            let mut result = DensePolynomial {
                coeffs: self
                    .coeffs
                    .iter()
                    .zip(rhs.coeffs.iter())
                    .map(|(row_a, row_b)| {
                        row_a
                            .iter()
                            .zip(row_b.iter())
                            .map(|(a, b)| *a + *b)
                            .collect()
                    })
                    .collect(),
                deg_x: self.deg_x,
                deg_y: self.deg_y,
            };
            result.update_degree();
            result
        } else {
            let deg_x = self.deg_x.max(rhs.deg_x);
            let deg_y = self.deg_y.max(rhs.deg_y);

            let mut sum_coeffs = vec![vec![F::default(); deg_y + 1]; deg_x + 1];
            sum_coeffs
                .iter_mut()
                .enumerate()
                .for_each(|(row_idx, row)| {
                    if row_idx <= self.deg_x {
                        row.iter_mut()
                            .zip(self.coeffs[row_idx].iter())
                            .for_each(|(a, b)| *a += b);
                    }
                    if row_idx <= rhs.deg_x {
                        row.iter_mut()
                            .zip(rhs.coeffs[row_idx].iter())
                            .for_each(|(a, b)| *a += b);
                    }
                });
            let mut result = DensePolynomial {
                coeffs: sum_coeffs,
                deg_x,
                deg_y,
            };
            result.update_degree();
            result
        }
    }
}

impl<'a, F: Field> AddAssign<&'a DensePolynomial<F>> for DensePolynomial<F> {
    #[inline]
    fn add_assign(&mut self, rhs: &'a DensePolynomial<F>) {
        if self.is_zero() {
            *self = rhs.clone();
        } else if rhs.is_zero() {
        } else if self.deg_x == rhs.deg_x && self.deg_y == rhs.deg_y {
            self.coeffs
                .iter_mut()
                .zip(rhs.coeffs.iter())
                .for_each(|(row_a, row_b)| {
                    row_a
                        .iter_mut()
                        .zip(row_b.iter())
                        .for_each(|(a, b)| *a += b);
                });
            self.update_degree();
        } else {
            let deg_x = self.deg_x.max(rhs.deg_x);
            let deg_y = self.deg_y.max(rhs.deg_y);

            // first resize self.coeffs to max of both dimension, pad with zeros
            if self.deg_x < deg_x {
                self.coeffs
                    .resize(deg_x + 1, vec![F::default(); self.deg_y + 1]);
            }
            if self.deg_y < deg_y {
                self.coeffs
                    .iter_mut()
                    .for_each(|row| row.resize(deg_y + 1, F::default()));
            }
            // go through rhs's rows to add each items
            self.coeffs
                .iter_mut()
                .take(rhs.deg_x + 1)
                .enumerate()
                .for_each(|(row_idx, row)| {
                    row.iter_mut()
                        .zip(rhs.coeffs[row_idx].iter())
                        .for_each(|(a, b)| *a += b)
                });
            self.deg_x = deg_x;
            self.deg_y = deg_y;
            self.update_degree();
        }
    }
}

impl<'a, F: Field> AddAssign<(F, &'a DensePolynomial<F>)> for DensePolynomial<F> {
    #[inline]
    fn add_assign(&mut self, (f, rhs): (F, &'a DensePolynomial<F>)) {
        if rhs.is_zero() || f.is_zero() {
        } else if self.is_zero() {
            *self = rhs.clone();
            self.coeffs
                .iter_mut()
                .for_each(|row| row.iter_mut().for_each(|c| *c *= &f));
            self.update_degree();
        } else {
            let deg_x = self.deg_x.max(rhs.deg_x);
            let deg_y = self.deg_y.max(rhs.deg_y);
            // first resize self.coeffs to max of both dimension, pad with zeros
            if self.deg_x < deg_x {
                self.coeffs
                    .resize(deg_x + 1, vec![F::default(); self.deg_y + 1]);
            }
            if self.deg_y < deg_y {
                self.coeffs
                    .iter_mut()
                    .for_each(|row| row.resize(deg_y + 1, F::default()));
            }
            // go through rhs's rows to scale-and-add each items
            self.coeffs
                .iter_mut()
                .take(rhs.deg_x + 1)
                .enumerate()
                .for_each(|(row_idx, row)| {
                    row.iter_mut()
                        .zip(rhs.coeffs[row_idx].iter())
                        .for_each(|(a, b)| *a += *b * &f)
                });
            self.deg_x = deg_x;
            self.deg_y = deg_y;
            self.update_degree();
        }
    }
}

impl<F: Field> Neg for DensePolynomial<F> {
    type Output = DensePolynomial<F>;
    #[inline]
    fn neg(mut self) -> Self::Output {
        self.coeffs.iter_mut().for_each(|row| {
            row.iter_mut().for_each(|c| *c = -*c);
        });
        self
    }
}

impl<'a, 'b, F: Field> Sub<&'a DensePolynomial<F>> for &'b DensePolynomial<F> {
    type Output = DensePolynomial<F>;
    #[inline]
    fn sub(self, rhs: &'a DensePolynomial<F>) -> Self::Output {
        if rhs.is_zero() {
            self.clone()
        } else if self.is_zero() {
            -rhs.clone()
        } else if self.deg_y == rhs.deg_y && self.deg_x == rhs.deg_x {
            let mut result = self.clone();
            result
                .coeffs
                .iter_mut()
                .zip(rhs.coeffs.iter())
                .for_each(|(row_a, row_b)| {
                    row_a
                        .iter_mut()
                        .zip(row_b.iter())
                        .for_each(|(a, b)| *a -= b)
                });
            result.update_degree();
            result
        } else {
            let deg_x = self.deg_x.max(rhs.deg_x);
            let deg_y = self.deg_y.max(rhs.deg_y);
            let mut coeffs = vec![vec![F::default(); deg_y + 1]; deg_x + 1];
            // 0 + self - rhs (on row that exists)
            coeffs.iter_mut().enumerate().for_each(|(row_idx, row)| {
                if row_idx <= self.deg_x {
                    row.iter_mut()
                        .zip(self.coeffs[row_idx].iter())
                        .for_each(|(a, b)| *a += b);
                }
                if row_idx <= rhs.deg_x {
                    row.iter_mut()
                        .zip(rhs.coeffs[row_idx].iter())
                        .for_each(|(a, b)| *a -= b);
                }
            });
            let mut result = DensePolynomial {
                coeffs,
                deg_x,
                deg_y,
            };
            result.update_degree();
            result
        }
    }
}

impl<'a, F: Field> SubAssign<&'a DensePolynomial<F>> for DensePolynomial<F> {
    #[inline]
    fn sub_assign(&mut self, rhs: &'a DensePolynomial<F>) {
        if rhs.is_zero() {
        } else if self.is_zero() {
            *self = -rhs.clone();
        } else if self.deg_y == rhs.deg_y && self.deg_x == rhs.deg_x {
            self.coeffs
                .iter_mut()
                .zip(rhs.coeffs.iter())
                .for_each(|(row_a, row_b)| {
                    row_a
                        .iter_mut()
                        .zip(row_b.iter())
                        .for_each(|(a, b)| *a -= b)
                });
            self.update_degree();
        } else {
            let deg_x = self.deg_x.max(rhs.deg_x);
            let deg_y = self.deg_y.max(rhs.deg_y);

            // first resize self.coeffs to max of both dimension, pad with zeros
            if self.deg_x < deg_x {
                self.coeffs
                    .resize(deg_x + 1, vec![F::default(); self.deg_y + 1]);
            }
            if self.deg_y < deg_y {
                self.coeffs
                    .iter_mut()
                    .for_each(|row| row.resize(deg_y + 1, F::default()));
            }

            // go through rhs's rows to subtract each items
            self.coeffs
                .iter_mut()
                .take(rhs.deg_x + 1)
                .enumerate()
                .for_each(|(row_idx, row)| {
                    row.iter_mut()
                        .zip(rhs.coeffs[row_idx].iter())
                        .for_each(|(a, b)| *a -= b);
                });
            self.deg_x = deg_x;
            self.deg_y = deg_y;
            self.update_degree();
        }
    }
}

impl<F: Field> Zero for DensePolynomial<F> {
    /// Returns the zero polynomial.
    fn zero() -> Self {
        Self {
            coeffs: vec![vec![F::ZERO]],
            deg_x: 0,
            deg_y: 0,
        }
    }

    /// Checks if the given polynomial is zero.
    fn is_zero(&self) -> bool {
        self.coeffs.is_empty() || self.coeffs.iter().flatten().all(|coeff| coeff.is_zero())
    }
}

impl<F: Field> Default for DensePolynomial<F> {
    fn default() -> Self {
        Self::zero()
    }
}

impl<F: Field> fmt::Debug for DensePolynomial<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            write!(f, "f(X,Y)=0")?;
            return Ok(());
        }
        let mut first_monomial_written = false;
        for (row_idx, row) in self.coeffs.iter().enumerate() {
            for (col_idx, cell) in row.iter().enumerate() {
                if !cell.is_zero() {
                    if first_monomial_written {
                        write!(f, " + {}*X^{}*Y^{}", cell, row_idx, col_idx)?;
                    } else {
                        write!(
                            f,
                            "f^[deg_x={}, deg_y={}](X,Y) = {}*X^{}*Y^{}",
                            self.deg_x, self.deg_y, cell, row_idx, col_idx
                        )?;
                        first_monomial_written = true;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ark_bls12_381::Fr;
    use ark_std::{test_rng, UniformRand};

    use super::*;

    #[test]
    fn add_polys() {
        let rng = &mut test_rng();
        for _ in 0..100 {
            let dx1 = rng.gen_range(5..20) as usize;
            let dy1 = rng.gen_range(5..20) as usize;
            let dx2 = rng.gen_range(5..20) as usize;
            let dy2 = rng.gen_range(5..20) as usize;
            let p1 = DensePolynomial::<Fr>::rand(dx1, dy1, rng);
            let p2 = DensePolynomial::<Fr>::rand(dx2, dy2, rng);
            assert_eq!(&p1 + &p2, &p2 + &p1);
        }
    }

    #[test]
    fn add_assign_polys() {
        let rng = &mut test_rng();
        for _ in 0..100 {
            let dx1 = rng.gen_range(5..20) as usize;
            let dy1 = rng.gen_range(5..20) as usize;
            let dx2 = rng.gen_range(5..20) as usize;
            let dy2 = rng.gen_range(5..20) as usize;

            let mut p1 = DensePolynomial::<Fr>::rand(dx1, dy1, rng);
            let p1_copy = p1.clone();
            let mut p2 = DensePolynomial::<Fr>::rand(dx2, dy2, rng);
            p1 += &p2;
            p2 += &p1_copy;
            assert_eq!(p1, p2);
        }
    }

    #[test]
    fn add_assign_scaled_polys() {
        let rng = &mut test_rng();
        for _ in 0..100 {
            let dx1 = rng.gen_range(5..20) as usize;
            let dy1 = rng.gen_range(5..20) as usize;
            let dx2 = rng.gen_range(5..20) as usize;
            let dy2 = rng.gen_range(5..20) as usize;
            let s = rng.gen_range(0..10);

            let mut p1 = DensePolynomial::<Fr>::rand(dx1, dy1, rng);
            let mut p1_copy = p1.clone();
            let p2 = DensePolynomial::<Fr>::rand(dx2, dy2, rng);

            for _ in 0..s {
                p1_copy += &p2;
            }
            p1 += (Fr::from(s), &p2);

            assert_eq!(p1, p1_copy);
        }
    }

    #[test]
    fn sub_polys() {
        let rng = &mut test_rng();
        for _ in 0..50 {
            let dx1 = rng.gen_range(5..20) as usize;
            let dy1 = rng.gen_range(5..20) as usize;
            let dx2 = rng.gen_range(5..20) as usize;
            let dy2 = rng.gen_range(5..20) as usize;

            let p1 = DensePolynomial::<Fr>::rand(dx1, dy1, rng);
            let p2 = DensePolynomial::<Fr>::rand(dx2, dy2, rng);
            let p1_minus_p2 = &p1 - &p2;
            let p2_minus_p1 = &p2 - &p1;
            assert_eq!(p1_minus_p2, -p2_minus_p1.clone());
            assert_eq!(&p1_minus_p2 + &p2, p1);
            assert_eq!(&p2_minus_p1 + &p1, p2);
        }
    }

    #[test]
    fn test_additive_identity() {
        // Test adding polynomials with its negative equals 0
        let rng = &mut test_rng();
        for _ in 0..10 {
            let dx1 = rng.gen_range(5..20) as usize;
            let dy1 = rng.gen_range(5..20) as usize;

            let p1 = DensePolynomial::<Fr>::rand(dx1, dy1, rng);
            assert_eq!(p1, -(-p1.clone()));
        }
    }

    #[test]
    fn evaluate_polys() {
        let rng = &mut test_rng();
        for _ in 0..100 {
            let dx = rng.gen_range(0..20) as usize;
            let dy = rng.gen_range(0..20) as usize;
            let point = (Fr::rand(rng), Fr::rand(rng));

            let p1 = DensePolynomial::<Fr>::rand(dx, dy, rng);
            let mut expected = Fr::zero();
            for (row_idx, row) in p1.coeffs.iter().enumerate() {
                for (col_idx, coeff) in row.iter().enumerate() {
                    expected +=
                        *coeff * point.0.pow(&[row_idx as u64]) * point.1.pow(&[col_idx as u64]);
                }
            }
            assert_eq!(p1.evaluate(&point), expected);
        }
    }

    #[test]
    fn partial_eval() {
        let rng = &mut test_rng();
        for _ in 0..100 {
            let dx = rng.gen_range(0..20) as usize;
            let dy = rng.gen_range(0..20) as usize;
            let x = Fr::rand(rng);
            let y = Fr::rand(rng);

            let p = DensePolynomial::<Fr>::rand(dx, dy, rng);
            let g_y = p.partial_evaluate_at_x(&x);
            let g_x = p.partial_evaluate_at_y(&y);
            assert_eq!(g_y.evaluate(&y), p.evaluate(&(x, y)));
            assert_eq!(g_x.evaluate(&x), p.evaluate(&(x, y)));
        }
    }

    #[test]
    fn div_polys() {
        let rng = &mut test_rng();
        for _ in 0..100 {
            let dx = rng.gen_range(0..20) as usize;
            let dy = rng.gen_range(0..20) as usize;
            // we allow larger than dx degree
            let divisor_deg = rng.gen_range(0..dx + 5) as usize;

            let p = DensePolynomial::<Fr>::rand(dx, dy, rng);
            let divisor = univariate::DensePolynomial::rand(divisor_deg, rng);
            let in_x = true;
            if divisor.is_zero() {
                continue;
            }
            let (quotient, remainder) = p.divide_with_q_and_r(&divisor, in_x).unwrap();

            // since we didn't implement mixed-mul of bivariate and univariate f(X,Y) * g(X)
            // we test "quotient * divisor + remainder = original" via evaluations at random
            // points
            let r = (Fr::rand(rng), Fr::rand(rng));
            assert_eq!(
                quotient.evaluate(&r) * divisor.evaluate(&r.0) + remainder.evaluate(&r),
                p.evaluate(&r)
            );

            // now test against y
            let in_x = false;
            let (quotient, remainder) = p.divide_with_q_and_r(&divisor, in_x).unwrap();
            assert_eq!(
                quotient.evaluate(&r) * divisor.evaluate(&r.1) + remainder.evaluate(&r),
                p.evaluate(&r)
            );
        }
    }

    #[test]
    fn sub_assign_uv_polys() {
        let rng = &mut test_rng();
        for _ in 0..100 {
            let dx = rng.gen_range(1..20) as usize;
            let dy = rng.gen_range(1..20) as usize;
            let uv_deg = rng.gen_range(0..dx) as usize;

            let mut p = DensePolynomial::<Fr>::rand(dx, dy, rng);
            let p_old = p.clone();
            let q_x = univariate::DensePolynomial::rand(uv_deg, rng);
            p.sub_assign_uv_poly(&q_x, true);

            // we test p' = p-q via evaluations at random points
            let r = (Fr::rand(rng), Fr::rand(rng));
            assert_eq!(p.evaluate(&r), p_old.evaluate(&r) - q_x.evaluate(&r.0));
            p.sub_assign_uv_poly(&-q_x, true);
            assert_eq!(p, p_old);

            let p_old = p.clone();
            let uv_deg = rng.gen_range(0..dy) as usize;
            let q_y = univariate::DensePolynomial::rand(uv_deg, rng);
            p.sub_assign_uv_poly(&q_y, false);
            assert_eq!(p.evaluate(&r), p_old.evaluate(&r) - q_y.evaluate(&r.1));
            p.sub_assign_uv_poly(&-q_y, false);
            assert_eq!(p, p_old);
        }
    }

    #[test]
    fn update_degree() {
        let mut p = DensePolynomial {
            coeffs: vec![vec![Fr::ONE, Fr::ONE], vec![Fr::ZERO, Fr::ZERO]],
            deg_x: 1,
            deg_y: 1,
        };
        p.update_degree();
        assert!(p.deg_x == 0 && p.deg_y == 1);
        let mut p = DensePolynomial {
            coeffs: vec![
                vec![Fr::ONE, Fr::ONE],
                vec![Fr::ZERO, Fr::ZERO],
                vec![Fr::ZERO, Fr::ZERO],
                vec![Fr::ZERO, Fr::ZERO],
            ],
            deg_x: 3,
            deg_y: 1,
        };
        p.update_degree();
        assert!(p.deg_x == 0 && p.deg_y == 1);
        let mut p = DensePolynomial {
            coeffs: vec![
                vec![Fr::ONE, Fr::ZERO],
                vec![Fr::ZERO, Fr::ZERO],
                vec![Fr::ZERO, Fr::ZERO],
            ],
            deg_x: 2,
            deg_y: 1,
        };
        p.update_degree();
        assert!(p.deg_x == 0 && p.deg_y == 0);
        let mut p = DensePolynomial {
            coeffs: vec![
                vec![Fr::ONE, Fr::ZERO],
                vec![Fr::ZERO, Fr::ZERO],
                vec![Fr::ZERO, Fr::ONE],
            ],
            deg_x: 2,
            deg_y: 1,
        };
        p.update_degree();
        assert!(p.deg_x == 2 && p.deg_y == 1);
        let mut p = DensePolynomial {
            coeffs: vec![vec![Fr::ZERO, Fr::ZERO], vec![Fr::ZERO, Fr::ZERO]],
            deg_x: 1,
            deg_y: 1,
        };
        p.update_degree();
        assert!(p.deg_x == 0 && p.deg_y == 0 && p.is_zero());
    }
}
