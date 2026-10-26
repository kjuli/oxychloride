//! Tests of reading and writing `.henshin` files.

use oxy_ecore::Metamodel;

use super::*;
use crate::Module;
use crate::print::Printer;

const BANK: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="bank" nsURI="http://bank" nsPrefix="bank">
  <eClassifiers xsi:type="ecore:EClass" name="Bank">
    <eStructuralFeatures xsi:type="ecore:EReference" name="clients" upperBound="-1" eType="#//Client" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="accounts" upperBound="-1" eType="#//Account" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="managers" upperBound="-1" eType="#//Manager" containment="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Person" abstract="true">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="name" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Client" eSuperTypes="#//Person">
    <eStructuralFeatures xsi:type="ecore:EReference" name="accounts" upperBound="-1" eType="#//Account" eOpposite="#//Account/owner"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Manager" eSuperTypes="#//Person">
    <eStructuralFeatures xsi:type="ecore:EReference" name="clients" upperBound="-1" eType="#//Client"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Account">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="id" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="credit" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EDouble"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="kind" eType="#//Kind"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="owner" eType="#//Client" eOpposite="#//Client/accounts"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EEnum" name="Kind">
    <eLiterals name="SAVINGS"/>
    <eLiterals name="CHECKING" value="1"/>
  </eClassifiers>
</ecore:EPackage>"##;

fn bank() -> Metamodel {
    let mut mm = Metamodel::new();
    mm.load_ecore_str(BANK, "/models/bank.ecore").unwrap();
    mm
}

const RULES: &str = r#"ePackageImport bank

// Comments are not part of the model.
rule createAccount(IN client:EString, IN accountId:EInt) {
	javaImport java.lang
	graph {
		node bank:Bank
		node owner:Client {
			name=client
		}
		create node account:Account {
			id=accountId
			credit=0.0
			kind=Kind.SAVINGS
		}
		forbid node existing:Account {
			id=accountId
		}
		edges [(bank->owner:clients), (create bank->account:accounts), (create owner->account:accounts), (forbid bank->existing:accounts)]
	}
}

rule transfer(IN client:EString, IN fromId:EInt, IN toId:EInt, IN amount:EDouble, VAR x:EDouble, VAR y:EDouble) {
	conditions [x > amount, amount > 0]
	graph {
		node from:Account {
			id=fromId
			credit=x
			set credit=x - amount
		}
		node to:Account {
			id=toId
			credit=y
			set credit=y + amount
			delete kind=Kind.CHECKING
		}
		node c:Client {
			name=client
			require name="vip"
		}
		require node m:Manager
		edges [(c->from:accounts), (require m->c:clients)]
	}
}

rule deleteAllAccounts(IN client:EString) {
	checkDangling false
	injectiveMatching false
	graph {
		node c:Client {
			name=client
		}
		node bank:Bank
		edges [(bank->c:clients)]
		multiRule accounts {
			graph {
				delete node a:Account
				reuse c {
					forbid name="x"
				}
				edges [(delete c->a:accounts), (delete bank->a:accounts)]
			}
		}
		matchingFormula {
			formula !hasManager OR (rich XOR !poor)
			conditionGraph hasManager {
				node m:Manager
				edges [(m->c:clients)]
			}
			conditionGraph rich {
				node a:Account {
					credit=1000000.0
				}
				edges [(c->a:accounts)]
				matchingFormula {
					formula !other
					conditionGraph other {
						node o:Account
						reuse a {
							id=1
						}
						edges [(bank->o:accounts)]
					}
				}
			}
			conditionGraph poor {
				reuse c {
					name="poor"
				}
			}
		}
	}
}

unit openAccounts(IN client:EString, IN a:EInt, IN b:EInt) {
	strict false
	createAccount(client, a)
	createAccount(client, b)
	{
		createAccount(client, a)
		deleteAllAccounts(client)
	}
	while {
		deleteAllAccounts(client)
	}
}

unit choose(IN client:EString, IN a:EInt) {
	priority [createAccount(client, a), deleteAllAccounts(client) createAccount(client, a)]
}

unit repeat(IN client:EString, IN n:EInt) {
	for (n - 1) {
		deleteAllAccounts(client)
	}
}

unit decide(IN client:EString, IN a:EInt) {
	if (createAccount(client, a)) then {
		deleteAllAccounts(client)
	} else {
		independent [openAccounts(client, a, a), choose(client, a) repeat(client, a)]
	}
}

unit weird(IN client:EString) {
	createAccount(client, javascriptCount)
}
"#;

fn compile(mm: &Metamodel, text: &str) -> Module {
    let m = Module::compile(mm, text);
    let errors: Vec<_> = m.problems.iter().filter(|p| p.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    m
}

fn options(template: Option<&Template>) -> WriteOptions<'_> {
    WriteOptions { path: "/models/bank.henshin", name: None, template }
}

/// The rules without the unit that has errors.
fn rules() -> String {
    RULES[..RULES.find("unit weird").unwrap()].to_string()
}

#[test]
fn writes_and_reads_the_same_module() {
    let mm = bank();
    let module = compile(&mm, &rules());
    let xmi = write(&mm, &module, &options(None));
    let read = read(&mm, &xmi, "/models/bank.henshin");
    assert!(read.problems.is_empty(), "{:?}", read.problems);
    let text = Printer::default().document(&read.document);
    let again = compile(&mm, &text);
    // Equivalent: the same file again.
    assert_eq!(write(&mm, &again, &options(None)), xmi, "{text}");
    // And the text is stable.
    let reread = read_text(&mm, &write(&mm, &again, &options(Some(&read.template))));
    assert_eq!(reread, text);
    // The parts of the text are there.
    for part in [
        "\t\tforbid node existing:Account {\n\t\t\tid=accountId\n\t\t}",
        "set credit=x - amount",
        "delete kind=Kind.CHECKING",
        "require name=\"vip\"",
        "(require m->c:clients)",
        "multiRule accounts {",
        "reuse c {\n\t\t\t\t\tforbid name=\"x\"",
        "formula !hasManager OR (rich XOR !poor)",
        "\t\t\t\t\tformula !other",
        "\tstrict false\n",
        "\twhile {\n\t\tdeleteAllAccounts(client)\n\t}",
        "\t{\n\t\tcreateAccount(client, a)\n\t\tdeleteAllAccounts(client)\n\t}",
        "priority [createAccount(client, a), deleteAllAccounts(client) createAccount(client, a)]",
        "for (n - 1) {",
        "javaImport java.lang",
        "checkDangling false\n\tinjectiveMatching false",
    ] {
        assert!(text.contains(part), "missing {part:?} in\n{text}");
    }
}

fn read_text(mm: &Metamodel, xmi: &str) -> String {
    let r = read(mm, xmi, "/models/bank.henshin");
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    Printer::default().document(&r.document)
}

#[test]
fn writes_henshin_models() {
    let mm = bank();
    let module = compile(&mm, &rules());
    let xmi = write(&mm, &module, &options(None));
    assert!(
        xmi.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<henshin:Module xmi:version=\"2.0\""),
        "{xmi}"
    );
    assert!(xmi.contains(" name=\"bank\""));
    assert!(xmi.contains("<imports href=\"bank.ecore#/\"/>"), "{xmi}");
    assert!(xmi.contains("<units xsi:type=\"henshin:Rule\""));
    // Types by reference into the .ecore file, attribute values as JavaScript.
    assert!(xmi.contains("<type href=\"bank.ecore#//Account\"/>"));
    assert!(
        xmi.contains("<type xsi:type=\"ecore:EDataType\" href=\"http://www.eclipse.org/emf/2002/Ecore#//EString\"/>")
    );
    assert!(xmi.contains("value=\"x-amount\""));
    assert!(xmi.contains("value=\"&quot;SAVINGS&quot;\""));
    assert!(xmi.contains("conditionText=\"x&gt;amount\""));
    // The negative condition, and the nested blocks of units as units of their own.
    assert!(xmi.contains("<formula xsi:type=\"henshin:Not\"") && xmi.contains("<formula xsi:type=\"henshin:Or\""));
    assert!(
        xmi.contains("<right xsi:type=\"henshin:Xor\"")
            && xmi.contains("<formula xsi:type=\"henshin:NestedCondition\"")
    );
    assert!(xmi.contains("<conclusion xmi:id=") && xmi.contains("name=\"forbid\""));
    assert!(xmi.contains("<units xsi:type=\"henshin:SequentialUnit\"") && xmi.contains("name=\"openAccounts_3\""));
    assert!(xmi.contains(&format!("key=\"{NESTED}\" value=\"block\"")));
    assert!(xmi.contains("<units xsi:type=\"henshin:LoopUnit\"") && xmi.contains("name=\"openAccounts_4\""));
    assert!(xmi.contains("<units xsi:type=\"henshin:IteratedUnit\"") && xmi.contains("iterations=\"n-1\""));
    // Ids are stable.
    assert_eq!(write(&mm, &module, &options(None)), xmi);
    assert_eq!(derived_id("x").len(), 23);
}

#[test]
fn keeps_what_the_text_cannot_express() {
    let mm = bank();
    let module = compile(&mm, &rules());
    let xmi = write(&mm, &module, &options(None));
    // Someone adds a description and an annotation, and changes an id.
    let edited = xmi
        .replacen("name=\"createAccount\"", "name=\"createAccount\" description=\"Opens an account.\"", 1)
        .replacen("\" conditionText=", "\" name=\"covered\" conditionText=", 1);
    let first_unit_id = {
        let at = edited.find("name=\"createAccount\"").unwrap();
        let start = edited[..at].rfind("xmi:id=\"").unwrap() + 8;
        edited[start..start + 23].to_string()
    };
    let edited = edited.replace(&first_unit_id, "_custom");
    let read = read(&mm, &edited, "/models/bank.henshin");
    assert!(read.problems.is_empty(), "{:?}", read.problems);
    let module = compile(&mm, &Printer::default().document(&read.document));
    let again = write(&mm, &module, &options(Some(&read.template)));
    assert_eq!(again, edited);
}

#[test]
fn reads_files_of_henshins_editors() {
    let mm = bank();
    // As Henshin's graphical editor writes them: unnamed nodes, a NAC without a name, a
    // multi-rule with copies of the kernel's nodes and edges.
    let file = r#"<?xml version="1.0" encoding="UTF-8"?>
<henshin:Module xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" xmlns:henshin="http://www.eclipse.org/emf/2011/Henshin" xmi:id="_m">
  <imports href="bank.ecore#/"/>
  <units xsi:type="henshin:Rule" xmi:id="_r1" name="createAccount">
    <parameters xmi:id="_p1" name="accountId" kind="IN">
      <type xsi:type="ecore:EDataType" href="http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
    </parameters>
    <lhs xmi:id="_l1" name="LHS">
      <nodes xmi:id="_n1">
        <type href="bank.ecore#//Bank"/>
      </nodes>
      <formula xsi:type="henshin:Not" xmi:id="_f1">
        <child xsi:type="henshin:NestedCondition" xmi:id="_c1">
          <conclusion xmi:id="_g1" name="">
            <nodes xmi:id="_n2" outgoing="_e1">
              <type href="bank.ecore#//Bank"/>
            </nodes>
            <nodes xmi:id="_n3" incoming="_e1">
              <type href="bank.ecore#//Account"/>
              <attributes xmi:id="_a1" value="accountId">
                <type href="bank.ecore#//Account/id"/>
              </attributes>
            </nodes>
            <edges xmi:id="_e1" source="_n2" target="_n3">
              <type href="bank.ecore#//Bank/accounts"/>
            </edges>
          </conclusion>
          <mappings xmi:id="_mp1" origin="_n1" image="_n2"/>
        </child>
      </formula>
    </lhs>
    <rhs xmi:id="_r1r" name="RHS">
      <nodes xmi:id="_n4" outgoing="_e2">
        <type href="bank.ecore#//Bank"/>
      </nodes>
      <nodes xmi:id="_n5" incoming="_e2">
        <type href="bank.ecore#//Account"/>
        <attributes xmi:id="_a2" value="accountId">
          <type href="bank.ecore#//Account/id"/>
        </attributes>
        <attributes xmi:id="_a3" value="&quot;CHECKING&quot;">
          <type href="bank.ecore#//Account/kind"/>
        </attributes>
        <attributes xmi:id="_a4" value="accountId &gt; 3 ? 1.0 : 0.0">
          <type href="bank.ecore#//Account/credit"/>
        </attributes>
      </nodes>
      <edges xmi:id="_e2" source="_n4" target="_n5">
        <type href="bank.ecore#//Bank/accounts"/>
      </edges>
    </rhs>
    <mappings xmi:id="_mp2" origin="_n1" image="_n4"/>
    <multiRules xmi:id="_mr" name="">
      <parameters xmi:id="_p2" name="accountId" kind="IN">
        <type xsi:type="ecore:EDataType" href="http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
      </parameters>
      <lhs xmi:id="_ml" name="LHS">
        <nodes xmi:id="_mn1" outgoing="_me1">
          <type href="bank.ecore#//Bank"/>
        </nodes>
        <nodes xmi:id="_mn2" incoming="_me1">
          <type href="bank.ecore#//Client"/>
        </nodes>
        <edges xmi:id="_me1" source="_mn1" target="_mn2">
          <type href="bank.ecore#//Bank/clients"/>
        </edges>
      </lhs>
      <rhs xmi:id="_mr2" name="RHS">
        <nodes xmi:id="_mn3">
          <type href="bank.ecore#//Bank"/>
        </nodes>
      </rhs>
      <mappings xmi:id="_mm1" origin="_mn1" image="_mn3"/>
      <multiMappings xmi:id="_mm2" origin="_n1" image="_mn1"/>
      <multiMappings xmi:id="_mm3" origin="_n4" image="_mn3"/>
    </multiRules>
  </units>
  <units xsi:type="henshin:SequentialUnit" xmi:id="_u1" name="twice" subUnits="_r1 _r1">
    <parameters xmi:id="_up" name="id" kind="IN">
      <type xsi:type="ecore:EDataType" href="http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
    </parameters>
    <parameterMappings xmi:id="_pm1" source="_up" target="_p1"/>
  </units>
  <units xsi:type="henshin:LoopUnit" xmi:id="_u2" name="forever" subUnit="_r1"/>
</henshin:Module>
"#;
    let r = read(&mm, file, "/models/bank.henshin");
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    let text = Printer::default().document(&r.document);
    let module = Module::compile(&mm, &text);
    let errors: Vec<_> = module.problems.iter().filter(|p| p.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}\n{text}");
    for part in [
        "rule createAccount(IN accountId:EInt) {",
        "\t\tnode bank:Bank\n",
        "\t\tcreate node account:Account {\n\t\t\tid=accountId\n\t\t\tkind=Kind.CHECKING\n\t\t\tcredit=javascript(\"accountId > 3 ? 1.0 : 0.0\")\n\t\t}",
        "\t\tforbid node account2:Account {\n\t\t\tid=accountId\n\t\t}",
        "edges [(create bank->account:accounts), (forbid bank->account2:accounts)]",
        "\t\tmultiRule multi {\n\t\t\tgraph {\n\t\t\t\tdelete node client:Client\n\t\t\t\tedges [(delete bank->client:clients)]",
        "unit twice(IN id:EInt) {\n\tcreateAccount(id)\n\tcreateAccount(id)\n}",
        // A parameter without a mapping gets a variable.
        "unit forever(VAR accountId:EInt) {\n\twhile {\n\t\tcreateAccount(accountId)\n\t}\n}",
    ] {
        assert!(text.contains(part), "missing {part:?} in\n{text}");
    }
    // The JavaScript the text cannot express goes back as it was.
    let xmi = write(&mm, &module, &options(Some(&r.template)));
    assert!(xmi.contains("value=\"accountId &gt; 3 ? 1.0 : 0.0\""), "{xmi}");
    // The editor's ids are kept, and so are the graphs' names.
    assert!(xmi.contains("<units xsi:type=\"henshin:Rule\" xmi:id=\"_r1\" name=\"createAccount\""), "{xmi}");
    assert!(xmi.contains("<lhs xmi:id=\"_l1\" name=\"LHS\">"), "{xmi}");
}

#[test]
fn reports_problems_in_the_file() {
    let mm = bank();
    let r = read(&mm, "<henshin:Module", "x.henshin");
    assert_eq!(r.problems.len(), 1);
    let r = read(&mm, "<?xml version=\"1.0\"?><ecore:EPackage/>", "x.henshin");
    assert!(r.problems[0].message.contains("not a Henshin module"));
    let file = r#"<henshin:Module xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:henshin="http://www.eclipse.org/emf/2011/Henshin">
  <imports href="bank.ecore#/"/>
  <units xsi:type="henshin:Rule" xmi:id="_r" name="r">
    <lhs xmi:id="_l"><nodes xmi:id="_n"><type href="bank.ecore#//Nope"/></nodes></lhs>
    <rhs xmi:id="_rr"/>
  </units>
  <units xsi:type="henshin:SequentialUnit" xmi:id="_u" name="u" subUnits="_missing"/>
</henshin:Module>"#;
    let r = read(&mm, file, "/models/x.henshin");
    assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
    assert_eq!(&file[r.problems[0].span.clone()], "_missing");
    // The compiler reports the unknown class, at its reference.
    let module = Module::from_document(&mm, &r.document, r.problems);
    let unknown = module.problems.iter().find(|p| p.message.contains("unknown class")).unwrap();
    assert_eq!(&file[unknown.span.clone()], "bank.ecore#//Nope");
}
