#![allow(clippy::many_single_char_names)]
#![allow(clippy::op_ref)]

use halo2::arithmetic::FieldExt;
use halo2::circuit::{Chip, Layouter, Region};
use halo2::plonk::{Advice, Column, ConstraintSystem, Error, Selector, TableColumn};
use halo2::poly::Rotation;
use std::marker::PhantomData;

// | A   | B   | C   | D       |
// | --- | --- | --- | ------- |
// |     |     |     | d_(i-1) |
// | a_i | b_i | c_i | d_i     |

// __Goal__:
// b: bit len of a limb

// * `a_i + b_i << b + c_i << 2b + d_i << 3b == d_(i-1)`
// * `a_i < 2^b`, `b_i < 2^b`, `c_i < 2^b`, `d_i < 2^b`

const LIMB_SIZE: usize = 4;

#[derive(Copy, Clone, Debug)]
pub struct Variable(Column<Advice>, usize);

#[derive(Clone, Debug)]
pub struct RangeConfig<F: FieldExt> {
    a: Column<Advice>,
    b: Column<Advice>,
    c: Column<Advice>,
    d: Column<Advice>,
    s_range: Selector,
    small_range_table: TableColumn,

    small_range_table_values: Vec<F>,
}

trait RangeInstructions<FF: FieldExt>: Chip<FF> {
    fn load_small_range_table(&self, layouter: &mut impl Layouter<FF>) -> Result<(), Error>;

    fn decomposition(
        &self,
        region: &mut Region<'_, FF>,
        value_integer: Option<FF>,
        value_limbs: Option<[FF; LIMB_SIZE]>,
    ) -> Result<(), Error>;
}

pub struct RangeChip<F: FieldExt, const BASE: usize> {
    config: RangeConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: FieldExt, const BASE: usize> Chip<F> for RangeChip<F, BASE> {
    type Config = RangeConfig<F>;
    type Loaded = ();

    fn config(&self) -> &Self::Config {
        &self.config
    }

    fn loaded(&self) -> &Self::Loaded {
        &()
    }
}

impl<FF: FieldExt, const BASE: usize> RangeInstructions<FF> for RangeChip<FF, BASE> {
    fn decomposition(
        &self,
        mut region: &mut Region<'_, FF>,
        value_integer: Option<FF>,
        value_limbs: Option<[FF; LIMB_SIZE]>,
    ) -> Result<(), Error> {
        let offset_integer = 0;
        let offset_limbs = offset_integer + 1;

        self.config.s_range.enable(&mut region, offset_limbs)?;

        let zero = FF::zero();
        let _ = region.assign_advice(|| "0 a", self.config.a, 0, || Ok(zero))?;
        let _ = region.assign_advice(|| "0 b", self.config.b, 0, || Ok(zero))?;
        let _ = region.assign_advice(|| "0 c", self.config.c, 0, || Ok(zero))?;
        let _ = region.assign_advice(
            || "integer",
            self.config.d,
            offset_integer,
            || Ok(value_integer.ok_or(Error::SynthesisError)?),
        )?;
        let _ = region.assign_advice(
            || "limb 0",
            self.config.a,
            offset_limbs,
            || Ok(value_limbs.ok_or(Error::SynthesisError)?[0]),
        )?;
        let _ = region.assign_advice(
            || "limb 1",
            self.config.b,
            offset_limbs,
            || Ok(value_limbs.ok_or(Error::SynthesisError)?[1]),
        )?;
        let _ = region.assign_advice(
            || "limb 2",
            self.config.c,
            offset_limbs,
            || Ok(value_limbs.ok_or(Error::SynthesisError)?[2]),
        )?;
        let _ = region.assign_advice(
            || "limb 3",
            self.config.d,
            offset_limbs,
            || Ok(value_limbs.ok_or(Error::SynthesisError)?[3]),
        )?;
        Ok(())
    }

    fn load_small_range_table(&self, layouter: &mut impl Layouter<FF>) -> Result<(), Error> {
        layouter.assign_table(
            || "",
            |mut table| {
                for (index, &value) in self.config.small_range_table_values.iter().enumerate() {
                    table.assign_cell(
                        || "small range table",
                        self.config.small_range_table,
                        index,
                        || Ok(value),
                    )?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}

impl<F: FieldExt, const BASE: usize> RangeChip<F, BASE> {
    pub fn new(config: RangeConfig<F>) -> Self {
        RangeChip {
            config,
            _marker: PhantomData,
        }
    }

    pub fn configure(
        meta: &mut ConstraintSystem<F>,
        limbs: [Column<Advice>; LIMB_SIZE],
    ) -> RangeConfig<F> {
        let small_range_table_values: Vec<F> = (0..1 << BASE).map(|e| F::from_u64(e)).collect();

        let a = limbs[0];
        let b = limbs[1];
        let c = limbs[2];
        let d = limbs[3];

        let s_range = meta.complex_selector();
        let small_range_table = meta.lookup_table_column();

        meta.lookup(|meta| {
            let a_ = meta.query_advice(a.into(), Rotation::cur());
            let s_range = meta.query_selector(s_range);
            vec![(a_ * s_range, small_range_table)]
        });

        meta.lookup(|meta| {
            let b_ = meta.query_advice(b.into(), Rotation::cur());
            let s_range = meta.query_selector(s_range);
            vec![(b_ * s_range, small_range_table)]
        });

        meta.lookup(|meta| {
            let c_ = meta.query_advice(c.into(), Rotation::cur());
            let s_range = meta.query_selector(s_range);
            vec![(c_ * s_range, small_range_table)]
        });

        meta.lookup(|meta| {
            let d_ = meta.query_advice(c.into(), Rotation::cur());
            let s_range = meta.query_selector(s_range);
            vec![(d_ * s_range, small_range_table)]
        });

        meta.create_gate("range", |meta| {
            let s_range = meta.query_selector(s_range);

            let a = meta.query_advice(a, Rotation::cur());
            let b = meta.query_advice(b, Rotation::cur());
            let c = meta.query_advice(c, Rotation::cur());
            let d_next = meta.query_advice(d, Rotation::prev());
            let d = meta.query_advice(d, Rotation::cur());

            let u1 = F::from_u64((1 << BASE) as u64);
            let u2 = F::from_u64((1 << (2 * BASE)) as u64);
            let u3 = F::from_u64((1 << (3 * BASE)) as u64);

            let expression = s_range * (a + b * u1 + c * u2 + d * u3 - d_next);
            vec![expression]
        });

        RangeConfig {
            a,
            b,
            c,
            d,
            s_range,
            small_range_table,

            small_range_table_values,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::{RangeChip, RangeConfig, RangeInstructions, LIMB_SIZE};
    use halo2::arithmetic::FieldExt;
    use halo2::circuit::{Layouter, SimpleFloorPlanner};
    use halo2::dev::MockProver;
    use halo2::pasta::Fp;
    use halo2::plonk::{Circuit, ConstraintSystem, Error};

    #[derive(Clone, Debug)]
    struct TestCircuitConfig<F: FieldExt> {
        range_config: RangeConfig<F>,
    }

    #[derive(Default)]
    struct TestCircuit<F: FieldExt, const BASE: usize> {
        integer: Option<F>,
    }

    impl<F: FieldExt, const BASE: usize> Circuit<F> for TestCircuit<F, BASE> {
        type Config = TestCircuitConfig<F>;
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            Self::default()
        }

        fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
            let a = meta.advice_column();
            let b = meta.advice_column();
            let c = meta.advice_column();
            let d = meta.advice_column();

            let range_config = RangeChip::<F, BASE>::configure(meta, [a, b, c, d]);
            TestCircuitConfig { range_config }
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<F>,
        ) -> Result<(), Error> {
            let decompose = |e: F, base: usize| -> [F; LIMB_SIZE] {
                use num_bigint::BigUint;
                let mut e = BigUint::from_bytes_le(&e.to_bytes()[..]);
                let n = (1 << base) as usize;
                let mut limbs: [F; LIMB_SIZE] = [F::zero(); LIMB_SIZE];
                for i in 0..LIMB_SIZE {
                    let u = BigUint::from(n - 1) & e.clone();
                    let u = F::from_str(&u.to_str_radix(10)).unwrap();
                    limbs[i] = u;
                    e = e >> base;
                }
                limbs
            };
            let range_chip = RangeChip::<F, BASE>::new(config.range_config);

            let integer = self.integer.ok_or(Error::SynthesisError)?;
            let limbs = decompose(integer, BASE);

            layouter.assign_region(
                || "decomposition",
                |mut region| {
                    range_chip.decomposition(&mut region, Some(integer), Some(limbs))?;
                    Ok(())
                },
            )?;

            range_chip.load_small_range_table(&mut layouter)?;

            Ok(())
        }
    }

    #[test]
    fn test_range() {
        const K: u32 = 5;
        const BASE: usize = 4;

        let integer = Some(Fp::from_u64(0xabcd));
        let circuit = TestCircuit::<Fp, BASE> { integer };

        let prover = match MockProver::run(K, &circuit, vec![]) {
            Ok(prover) => prover,
            Err(e) => panic!("{:#?}", e),
        };
        // println!("{:?}", prover);
        assert_eq!(prover.verify(), Ok(()));

        let integer = Some(Fp::from_u64(1 << (BASE * 4)));
        let circuit = TestCircuit::<Fp, BASE> { integer };

        let prover = match MockProver::run(K, &circuit, vec![]) {
            Ok(prover) => prover,
            Err(e) => panic!("{:#?}", e),
        };
        assert_ne!(prover.verify(), Ok(()));
    }
}
