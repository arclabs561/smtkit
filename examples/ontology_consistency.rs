//! Ontology consistency checking via SMT.
//!
//! Encodes EL++ ontology axioms as SMT constraints and uses Z3 to check
//! whether a set of assertions (concept inclusions, role assertions) is
//! satisfiable. If UNSAT, the ontology has a logical contradiction.
//!
//! Demonstrates the connection between smtkit and description logic /
//! knowledge graph reasoning.
//!
//! ## EL++ Axiom Types Encoded
//!
//! - Concept subsumption: A ⊑ B (every A is a B)
//! - Concept conjunction: A ⊓ B ⊑ C (things that are both A and B are C)
//! - Role inclusion: R ⊑ S (if R(x,y) then S(x,y))
//! - Disjointness: A ⊓ B ⊑ ⊥ (nothing is both A and B)

use smtkit::core::{Ctx, Op, Sort};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut ctx = Ctx::new();

    // Domain individuals (small finite domain for decidability).
    let n_individuals = 5;

    // Concept memberships: concept_A[i] = true iff individual i is in concept A.
    let animal: Vec<_> = (0..n_individuals)
        .map(|i| ctx.var(format!("animal_{i}"), Sort::Bool))
        .collect();
    let mammal: Vec<_> = (0..n_individuals)
        .map(|i| ctx.var(format!("mammal_{i}"), Sort::Bool))
        .collect();
    let bird: Vec<_> = (0..n_individuals)
        .map(|i| ctx.var(format!("bird_{i}"), Sort::Bool))
        .collect();
    let flyer: Vec<_> = (0..n_individuals)
        .map(|i| ctx.var(format!("flyer_{i}"), Sort::Bool))
        .collect();
    let penguin: Vec<_> = (0..n_individuals)
        .map(|i| ctx.var(format!("penguin_{i}"), Sort::Bool))
        .collect();

    let mut assertions = Vec::new();

    // Axiom 1: Mammal ⊑ Animal (every mammal is an animal)
    for i in 0..n_individuals {
        // mammal_i => animal_i
        let not_mammal = ctx.app(Op::Not, vec![mammal[i]]).unwrap();
        let impl_ax = ctx.app(Op::Or, vec![not_mammal, animal[i]]).unwrap();
        assertions.push(impl_ax);
    }

    // Axiom 2: Bird ⊑ Animal (every bird is an animal)
    for i in 0..n_individuals {
        let not_bird = ctx.app(Op::Not, vec![bird[i]]).unwrap();
        let impl_ax = ctx.app(Op::Or, vec![not_bird, animal[i]]).unwrap();
        assertions.push(impl_ax);
    }

    // Axiom 3: Penguin ⊑ Bird (every penguin is a bird)
    for i in 0..n_individuals {
        let not_pen = ctx.app(Op::Not, vec![penguin[i]]).unwrap();
        let impl_ax = ctx.app(Op::Or, vec![not_pen, bird[i]]).unwrap();
        assertions.push(impl_ax);
    }

    // Axiom 4: Bird ⊑ Flyer (every bird can fly) -- this is the problematic axiom
    for i in 0..n_individuals {
        let not_bird = ctx.app(Op::Not, vec![bird[i]]).unwrap();
        let impl_ax = ctx.app(Op::Or, vec![not_bird, flyer[i]]).unwrap();
        assertions.push(impl_ax);
    }

    // Axiom 5: Penguin ⊓ Flyer ⊑ ⊥ (penguins cannot fly -- disjointness)
    for i in 0..n_individuals {
        // NOT (penguin_i AND flyer_i)
        let conj = ctx.app(Op::And, vec![penguin[i], flyer[i]]).unwrap();
        let disjoint = ctx.app(Op::Not, vec![conj]).unwrap();
        assertions.push(disjoint);
    }

    // ABox assertion: individual 0 is a penguin.
    assertions.push(penguin[0]);

    // Check consistency: is there a model satisfying all axioms?
    let all_vars: Vec<_> = animal
        .iter()
        .chain(mammal.iter())
        .chain(bird.iter())
        .chain(flyer.iter())
        .chain(penguin.iter())
        .copied()
        .collect();

    println!("Ontology consistency check");
    println!(
        "  {} axiom instances, {} variables",
        assertions.len(),
        all_vars.len()
    );
    println!();
    println!("Axioms:");
    println!("  1. Mammal ⊑ Animal");
    println!("  2. Bird ⊑ Animal");
    println!("  3. Penguin ⊑ Bird");
    println!("  4. Bird ⊑ Flyer");
    println!("  5. Penguin ⊓ Flyer ⊑ ⊥");
    println!("  ABox: Penguin(ind_0)");
    println!();

    // This should be UNSAT because:
    // Penguin(0) => Bird(0) [axiom 3]
    // Bird(0) => Flyer(0) [axiom 4]
    // But Penguin(0) ∧ Flyer(0) => ⊥ [axiom 5]
    // Contradiction.

    #[cfg(feature = "z3-inproc")]
    {
        let result = smtkit::z3::solve_projected(&ctx, &assertions, &all_vars)?;
        match result.status {
            smtkit::SolveStatus::Sat => {
                println!("Result: SAT (ontology is consistent)");
                if let Some(model) = &result.model {
                    println!("Model: {model:?}");
                }
            }
            smtkit::SolveStatus::Unsat => {
                println!("Result: UNSAT (ontology has a contradiction)");
                println!();
                println!("Explanation: Penguin(0) implies Bird(0) [axiom 3],");
                println!("which implies Flyer(0) [axiom 4], but axiom 5 says");
                println!("no individual can be both a Penguin and a Flyer.");
            }
            smtkit::SolveStatus::Unknown => {
                println!("Result: UNKNOWN");
            }
        }
    }

    #[cfg(not(feature = "z3-inproc"))]
    {
        println!("(z3-inproc feature not enabled -- emitting SMT-LIB2 script instead)");
        println!();
        let mut script = smtkit::smt2::Script::new();
        script.set_logic("QF_LIA");
        for &v in &all_vars {
            if let smtkit::core::TermKind::Var { ref sym, .. } = ctx.kind_of(v) {
                // QF_LIA: every declared const is Int.
                let var = smtkit::smt2::Var::new(sym.0.clone(), smtkit::smt2::Sort::Int);
                script.declare_const(&var);
            }
        }
        for &a in &assertions {
            script.assert_term(&ctx, a);
        }
        script.check_sat();
        println!("{}", script);
    }

    Ok(())
}
