#[cfg(all(target_arch = "x86_64", feature = "deser"))]
use std::{hint::black_box, ops::Deref as _, sync::LazyLock};

#[cfg(all(target_arch = "x86_64", feature = "deser"))]
use groth16::{
    Groth16Proof, Groth16ProofJsonDeser, Groth16PublicInput, Groth16PublicInputDeser,
    Groth16VerificationKey, Groth16VerificationKeyJsonDeser, groth16_batch_verify, groth16_verify,
};
#[cfg(all(target_arch = "x86_64", feature = "deser"))]
use serde_json::{Value, json};

#[cfg(all(target_arch = "x86_64", feature = "deser"))]
static VK: LazyLock<Value> = LazyLock::new(|| {
    json!({
     "protocol": "groth16",
     "curve": "bn128",
     "nPublic": 3,
     "vk_alpha_1": [
      "20491192805390485299153009773594534940189261866228447918068658471970481763042",
      "9383485363053290200918347156157836566562967994039712273449902621266178545958",
      "1"
     ],
     "vk_beta_2": [
      [
       "6375614351688725206403948262868962793625744043794305715222011528459656738731",
       "4252822878758300859123897981450591353533073413197771768651442665752259397132"
      ],
      [
       "10505242626370262277552901082094356697409835680220590971873171140371331206856",
       "21847035105528745403288232691147584728191162732299865338377159692350059136679"
      ],
      [
       "1",
       "0"
      ]
     ],
     "vk_gamma_2": [
      [
       "10857046999023057135944570762232829481370756359578518086990519993285655852781",
       "11559732032986387107991004021392285783925812861821192530917403151452391805634"
      ],
      [
       "8495653923123431417604973247489272438418190587263600148770280649306958101930",
       "4082367875863433681332203403145435568316851327593401208105741076214120093531"
      ],
      [
       "1",
       "0"
      ]
     ],
     "vk_delta_2": [
      [
       "10857046999023057135944570762232829481370756359578518086990519993285655852781",
       "11559732032986387107991004021392285783925812861821192530917403151452391805634"
      ],
      [
       "8495653923123431417604973247489272438418190587263600148770280649306958101930",
       "4082367875863433681332203403145435568316851327593401208105741076214120093531"
      ],
      [
       "1",
       "0"
      ]
     ],
     "vk_alphabeta_12": [
      [
       [
        "2029413683389138792403550203267699914886160938906632433982220835551125967885",
        "21072700047562757817161031222997517981543347628379360635925549008442030252106"
       ],
       [
        "5940354580057074848093997050200682056184807770593307860589430076672439820312",
        "12156638873931618554171829126792193045421052652279363021382169897324752428276"
       ],
       [
        "7898200236362823042373859371574133993780991612861777490112507062703164551277",
        "7074218545237549455313236346927434013100842096812539264420499035217050630853"
       ]
      ],
      [
       [
        "7077479683546002997211712695946002074877511277312570035766170199895071832130",
        "10093483419865920389913245021038182291233451549023025229112148274109565435465"
       ],
       [
        "4595479056700221319381530156280926371456704509942304414423590385166031118820",
        "19831328484489333784475432780421641293929726139240675179672856274388269393268"
       ],
       [
        "11934129596455521040620786944827826205713621633706285934057045369193958244500",
        "8037395052364110730298837004334506829870972346962140206007064471173334027475"
       ]
      ]
     ],
     "IC": [
      [
       "3592500400225412638189588012691950104207241681364647224980389745358169964817",
       "17656989355482649231475742109448763213831201618717896520906546631001517299206",
       "1"
      ],
      [
       "16905092003921116564094294442987246461951065955646828320794879470136193192281",
       "20003748097236162141109193702759199323499512302110718878696307083233215239565",
       "1"
      ],
      [
       "2769706102616928116248644269748096090861058943116110168723685805629389007346",
       "18574056293516882592614575882335841310830140314560044201198030142558882757569",
       "1"
      ],
      [
       "18394754470378740764556529416208639614228313351963120985359200506531456485616",
       "19392296659829986162543487604543820064466904337122321509488947754894130933561",
       "1"
      ]
     ]
    })
});

#[cfg(all(target_arch = "x86_64", feature = "deser"))]
static PROOF: LazyLock<Value> = LazyLock::new(|| {
    json!({
      "pi_a": [
        "11037518239771719236357838810928088160115698859544072019676217195563812369339",
        "3687083306578953881426182275302586900896442322964634260086046492714156982737",
        "1"
      ],
      "pi_b": [
        [
          "15511436958914388193637325263170064059975425617162222638007974154758617626639",
          "12961273015405580998535601990621723818538326331554867910183898355414607244087"
        ],
        [
          "12814428998216757405035519489921355987283295280812705168125901147534571101205",
          "1082862524405828508471176354966812771158974036915692156538554700119208440965"
        ],
        [
          "1",
          "0"
        ]
      ],
      "pi_c": [
        "19297989616617856375592235181610057144521956015817110830875742229413585127103",
        "20376203207988572324788291551841864128517768060599431285032095684253198355005",
        "1"
      ],
      "protocol": "groth16",
      "curve": "bn128"
    })
});

#[cfg(all(target_arch = "x86_64", feature = "deser"))]
static PI: LazyLock<Value> = LazyLock::new(|| {
    json!([
        "18876773715140569969879940706526502898783381664583962060409399199413543299784",
        "21732699638645006545583066153007228756330057466373133431165650051520288853952",
        "21740391455879413819495285231787023297124882344149053147369278538770961315521"
    ])
});

// TODO: Remove this when we have the proper benches in the proofs
#[cfg(all(target_arch = "x86_64", feature = "deser"))]
#[expect(
    clippy::undocumented_unsafe_blocks,
    reason = "This test is is just to measure cpu and should be run manually"
)]
#[ignore = "This test is just for calculation the cycles for the above set of proofs. This will be moved to the pertinent proof in the future."]
#[test]
fn poc_cpu_cycles() {
    let proof: Groth16Proof =
        serde_json::from_value::<Groth16ProofJsonDeser>(PROOF.deref().clone())
            .unwrap()
            .try_into()
            .unwrap();
    let vk: Groth16VerificationKey =
        serde_json::from_value::<Groth16VerificationKeyJsonDeser>(VK.deref().clone())
            .unwrap()
            .try_into()
            .unwrap();
    let pi: Vec<_> = serde_json::from_value::<Vec<Groth16PublicInputDeser>>(PI.deref().clone())
        .unwrap()
        .into_iter()
        .map(TryInto::<Groth16PublicInput>::try_into)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(Groth16PublicInput::into_inner)
        .collect();
    let pvk = vk.into_prepared();
    let iters = 1000u64;
    let pre = unsafe { core::arch::x86_64::_rdtsc() };
    for _ in 0..iters {
        black_box(groth16_verify(&pvk, &proof, &pi).expect("success"));
    }
    let post = unsafe { core::arch::x86_64::_rdtsc() };
    let cycles = (post - pre) / iters;
    println!("This proof has {} public inputs", pi.len() - 1);
    println!("proof-of-claim-cycles-count: {cycles} cpu cycles");

    for batch_size in 1..10 {
        let proofs_batch: Vec<Groth16Proof> = std::iter::repeat_with(|| {
            serde_json::from_value::<Groth16ProofJsonDeser>(PROOF.deref().clone())
                .unwrap()
                .try_into()
                .unwrap()
        })
        .take(batch_size)
        .collect();

        let pi_batch: Vec<Vec<_>> = std::iter::repeat_with(|| pi.clone())
            .take(batch_size)
            .collect();
        let pre = unsafe { core::arch::x86_64::_rdtsc() };
        for _ in 0..iters {
            black_box(groth16_batch_verify(&pvk, &proofs_batch, &pi_batch));
        }
        let post = unsafe { core::arch::x86_64::_rdtsc() };
        let cycles = (post - pre) / iters;
        println!(
            "batched-proof-of-claim-cycles-count: {cycles} cpu cycles for batch {batch_size} batches"
        );
    }
    for batch_size in (10..201).step_by(10) {
        let proofs_batch: Vec<Groth16Proof> = std::iter::repeat_with(|| {
            serde_json::from_value::<Groth16ProofJsonDeser>(PROOF.deref().clone())
                .unwrap()
                .try_into()
                .unwrap()
        })
        .take(batch_size)
        .collect();

        let pi_batch: Vec<Vec<_>> = std::iter::repeat_with(|| pi.clone())
            .take(batch_size)
            .collect();
        let pre = unsafe { core::arch::x86_64::_rdtsc() };
        for _ in 0..iters {
            black_box(groth16_batch_verify(&pvk, &proofs_batch, &pi_batch));
        }
        let post = unsafe { core::arch::x86_64::_rdtsc() };
        let cycles = (post - pre) / iters;
        println!(
            "batched-proof-of-claim-cycles-count: {cycles} cpu cycles for batch {batch_size} batches"
        );
    }
}
