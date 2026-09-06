use ai_stock_forum::agents::builtin_profile_templates;

#[test]
fn every_builtin_template_has_exact_canonical_bytes_and_digest() {
    let expected = [
        (
            "builtin.bull",
            r#"{"description":"Develops evidence-led upside research.","id":"builtin.bull","instructions":"Develop the strongest evidence-backed bull case.","personality":"Constructive, precise, and evidence-led.","primary_specialty":"upside research","role":"bull","specialty_tags":["growth","catalysts"],"suggested_name":"Bull Researcher","version":1}"#,
            "b7cbf40db846e7396112213b3aa1db11ff7ffbcde71c3f04433929d5ad962c12",
        ),
        (
            "builtin.bear",
            r#"{"description":"Develops evidence-led downside research.","id":"builtin.bear","instructions":"Develop the strongest evidence-backed bear case.","personality":"Skeptical, precise, and evidence-led.","primary_specialty":"downside research","role":"bear","specialty_tags":["risk","valuation"],"suggested_name":"Bear Researcher","version":1}"#,
            "16b180a7e04d7203ff3f1404ebd66bcdbcd08f02c3f194c47ff275b639fcb5ca",
        ),
        (
            "builtin.chief",
            r#"{"description":"Synthesizes evidence into clear decisions.","id":"builtin.chief","instructions":"Arbitrate competing claims using cited evidence.","personality":"Balanced, rigorous, and decisive.","primary_specialty":"evidence synthesis","role":"chief","specialty_tags":["arbitration","decisions"],"suggested_name":"Chief Moderator","version":1}"#,
            "727573ee03a7ee15d3b6c750bc08f06369faafe15b69ed47c7bc0d68035a4246",
        ),
        (
            "builtin.engineering",
            r#"{"description":"Builds reliable systems for investment research.","id":"builtin.engineering","instructions":"Improve research tooling and data quality.","personality":"Methodical, practical, and quality-focused.","primary_specialty":"research systems","role":"engineering","specialty_tags":["tooling","data-quality"],"suggested_name":"Research Engineer","version":1}"#,
            "56ad50dce299049c7f1834025f96e661c4534d12582a8455f22f397c1df18139",
        ),
        (
            "builtin.custom",
            r#"{"description":"Provides a starting point for a custom analyst.","id":"builtin.custom","instructions":"","personality":"","primary_specialty":"","role":"custom","specialty_tags":[],"suggested_name":"Custom Analyst","version":1}"#,
            "aa866f4876c2b4c7312db7c97599bf9c1e4762409ee3e60c0e41a347f5b955ce",
        ),
    ];

    let templates = builtin_profile_templates();
    assert_eq!(templates.len(), expected.len());
    for (template, (id, bytes, digest)) in templates.iter().zip(expected) {
        assert_eq!(template.id.as_str(), id);
        assert_eq!(template.version.get(), 1);
        assert_eq!(template.canonical_bytes().unwrap(), bytes.as_bytes());
        assert_eq!(template.digest.as_str(), digest);
    }
}
