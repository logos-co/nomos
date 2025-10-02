pub mod enumerated {
    pub fn is_same_variant<T>(values: &[&T]) -> bool {
        let mut discriminants = values.iter().map(|item| std::mem::discriminant::<T>(item));
        let Some(head) = discriminants.next() else {
            return false;
        };
        discriminants.all(|tail_item| tail_item == head)
    }

    #[cfg(test)]
    mod tests {
        use crate::types::enumerated;

        enum TestEnum {
            A,
            B,
            C,
        }

        #[test]
        fn test_is_same_variant() {
            let values = [TestEnum::A, TestEnum::B, TestEnum::C];
            assert!(!enumerated::is_same_variant(&[
                &values[0], &values[1], &values[2]
            ]));

            let values = [TestEnum::A, TestEnum::A, TestEnum::A];
            assert!(enumerated::is_same_variant(&[
                &values[0], &values[1], &values[2]
            ]));
        }
    }
}
