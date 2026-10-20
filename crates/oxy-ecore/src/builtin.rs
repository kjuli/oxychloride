//! The Ecore metamodel itself, always present in a [`crate::Metamodel`].
//!
//! This is a trimmed-down `Ecore.ecore`: all classifiers and the non-derived structural
//! features, without generics details and Java-only operations.

pub const ECORE_ECORE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="ecore" nsURI="http://www.eclipse.org/emf/2002/Ecore" nsPrefix="ecore">
  <eClassifiers xsi:type="ecore:EClass" name="EAttribute" eSuperTypes="#//EStructuralFeature">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="iD" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eAttributeType" lowerBound="1" eType="#//EDataType" changeable="false" volatile="true" transient="true" derived="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EAnnotation" eSuperTypes="#//EModelElement">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="source" eType="#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="details" upperBound="-1" eType="#//EStringToStringMapEntry" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eModelElement" eType="#//EModelElement" transient="true" eOpposite="#//EModelElement/eAnnotations"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="contents" upperBound="-1" eType="#//EObject" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="references" upperBound="-1" eType="#//EObject"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EClass" eSuperTypes="#//EClassifier">
    <eOperations name="isSuperTypeOf" eType="#//EBoolean">
      <eParameters name="someClass" eType="#//EClass"/>
    </eOperations>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="abstract" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="interface" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eSuperTypes" upperBound="-1" eType="#//EClass" unsettable="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eOperations" upperBound="-1" eType="#//EOperation" containment="true" eOpposite="#//EOperation/eContainingClass"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eStructuralFeatures" upperBound="-1" eType="#//EStructuralFeature" containment="true" eOpposite="#//EStructuralFeature/eContainingClass"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eGenericSuperTypes" upperBound="-1" eType="#//EGenericType" containment="true" unsettable="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EClassifier" abstract="true" eSuperTypes="#//ENamedElement">
    <eOperations name="isInstance" eType="#//EBoolean">
      <eParameters name="object" eType="#//EJavaObject"/>
    </eOperations>
    <eOperations name="getClassifierID" eType="#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="instanceClassName" eType="#//EString" unsettable="true" volatile="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="instanceTypeName" eType="#//EString" unsettable="true" volatile="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="ePackage" eType="#//EPackage" changeable="false" transient="true" eOpposite="#//EPackage/eClassifiers"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eTypeParameters" upperBound="-1" eType="#//ETypeParameter" containment="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EDataType" eSuperTypes="#//EClassifier">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="serializable" eType="#//EBoolean" defaultValueLiteral="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EEnum" eSuperTypes="#//EDataType">
    <eStructuralFeatures xsi:type="ecore:EReference" name="eLiterals" upperBound="-1" eType="#//EEnumLiteral" containment="true" eOpposite="#//EEnumLiteral/eEnum"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EEnumLiteral" eSuperTypes="#//ENamedElement">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="value" eType="#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="literal" eType="#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eEnum" eType="#//EEnum" changeable="false" transient="true" eOpposite="#//EEnum/eLiterals"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EFactory" eSuperTypes="#//EModelElement">
    <eStructuralFeatures xsi:type="ecore:EReference" name="ePackage" lowerBound="1" eType="#//EPackage" transient="true" eOpposite="#//EPackage/eFactoryInstance"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EModelElement" abstract="true">
    <eOperations name="getEAnnotation" eType="#//EAnnotation">
      <eParameters name="source" eType="#//EString"/>
    </eOperations>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eAnnotations" upperBound="-1" eType="#//EAnnotation" containment="true" eOpposite="#//EAnnotation/eModelElement"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="ENamedElement" abstract="true" eSuperTypes="#//EModelElement">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="name" eType="#//EString"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EObject"/>
  <eClassifiers xsi:type="ecore:EClass" name="EOperation" eSuperTypes="#//ETypedElement">
    <eStructuralFeatures xsi:type="ecore:EReference" name="eContainingClass" eType="#//EClass" changeable="false" transient="true" eOpposite="#//EClass/eOperations"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eTypeParameters" upperBound="-1" eType="#//ETypeParameter" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eParameters" upperBound="-1" eType="#//EParameter" containment="true" eOpposite="#//EParameter/eOperation"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eExceptions" upperBound="-1" eType="#//EClassifier" unsettable="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eGenericExceptions" upperBound="-1" eType="#//EGenericType" containment="true" unsettable="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EPackage" eSuperTypes="#//ENamedElement">
    <eOperations name="getEClassifier" eType="#//EClassifier">
      <eParameters name="name" eType="#//EString"/>
    </eOperations>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="nsURI" eType="#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="nsPrefix" eType="#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eFactoryInstance" lowerBound="1" eType="#//EFactory" transient="true" eOpposite="#//EFactory/ePackage"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eClassifiers" upperBound="-1" eType="#//EClassifier" containment="true" eOpposite="#//EClassifier/ePackage"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eSubpackages" upperBound="-1" eType="#//EPackage" containment="true" eOpposite="#//EPackage/eSuperPackage"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eSuperPackage" eType="#//EPackage" changeable="false" transient="true" eOpposite="#//EPackage/eSubpackages"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EParameter" eSuperTypes="#//ETypedElement">
    <eStructuralFeatures xsi:type="ecore:EReference" name="eOperation" eType="#//EOperation" changeable="false" transient="true" eOpposite="#//EOperation/eParameters"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EReference" eSuperTypes="#//EStructuralFeature">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="containment" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="container" eType="#//EBoolean" changeable="false" volatile="true" transient="true" derived="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="resolveProxies" eType="#//EBoolean" defaultValueLiteral="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eOpposite" eType="#//EReference"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eReferenceType" lowerBound="1" eType="#//EClass" changeable="false" volatile="true" transient="true" derived="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eKeys" upperBound="-1" eType="#//EAttribute"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EStructuralFeature" abstract="true" eSuperTypes="#//ETypedElement">
    <eOperations name="getFeatureID" eType="#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="changeable" eType="#//EBoolean" defaultValueLiteral="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="volatile" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="transient" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="defaultValueLiteral" eType="#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="unsettable" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="derived" eType="#//EBoolean"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eContainingClass" eType="#//EClass" changeable="false" transient="true" eOpposite="#//EClass/eStructuralFeatures"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="ETypedElement" abstract="true" eSuperTypes="#//ENamedElement">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="ordered" eType="#//EBoolean" defaultValueLiteral="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="unique" eType="#//EBoolean" defaultValueLiteral="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="lowerBound" eType="#//EInt"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="upperBound" eType="#//EInt" defaultValueLiteral="1"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="many" eType="#//EBoolean" changeable="false" volatile="true" transient="true" derived="true"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="required" eType="#//EBoolean" changeable="false" volatile="true" transient="true" derived="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eType" eType="#//EClassifier" volatile="true" unsettable="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eGenericType" eType="#//EGenericType" volatile="true" containment="true" unsettable="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EStringToStringMapEntry" instanceClassName="java.util.Map$Entry">
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="key" eType="#//EString"/>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="value" eType="#//EString"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="EGenericType">
    <eStructuralFeatures xsi:type="ecore:EReference" name="eUpperBound" eType="#//EGenericType" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eTypeArguments" upperBound="-1" eType="#//EGenericType" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eLowerBound" eType="#//EGenericType" containment="true"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eTypeParameter" eType="#//ETypeParameter"/>
    <eStructuralFeatures xsi:type="ecore:EReference" name="eClassifier" eType="#//EClassifier"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="ETypeParameter" eSuperTypes="#//ENamedElement">
    <eStructuralFeatures xsi:type="ecore:EReference" name="eBounds" upperBound="-1" eType="#//EGenericType" containment="true"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EDataType" name="EBigDecimal" instanceClassName="java.math.BigDecimal"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EBigInteger" instanceClassName="java.math.BigInteger"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EBoolean" instanceClassName="boolean"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EBooleanObject" instanceClassName="java.lang.Boolean"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EByte" instanceClassName="byte"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EByteArray" instanceClassName="byte[]"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EByteObject" instanceClassName="java.lang.Byte"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EChar" instanceClassName="char"/>
  <eClassifiers xsi:type="ecore:EDataType" name="ECharacterObject" instanceClassName="java.lang.Character"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EDate" instanceClassName="java.util.Date"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EDiagnosticChain" instanceClassName="org.eclipse.emf.common.util.DiagnosticChain" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EDouble" instanceClassName="double"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EDoubleObject" instanceClassName="java.lang.Double"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EEList" instanceClassName="org.eclipse.emf.common.util.EList" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EEnumerator" instanceClassName="org.eclipse.emf.common.util.Enumerator" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EFeatureMap" instanceClassName="org.eclipse.emf.ecore.util.FeatureMap" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EFeatureMapEntry" instanceClassName="org.eclipse.emf.ecore.util.FeatureMap$Entry" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EFloat" instanceClassName="float"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EFloatObject" instanceClassName="java.lang.Float"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EInt" instanceClassName="int"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EIntegerObject" instanceClassName="java.lang.Integer"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EJavaClass" instanceClassName="java.lang.Class"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EJavaObject" instanceClassName="java.lang.Object"/>
  <eClassifiers xsi:type="ecore:EDataType" name="ELong" instanceClassName="long"/>
  <eClassifiers xsi:type="ecore:EDataType" name="ELongObject" instanceClassName="java.lang.Long"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EMap" instanceClassName="java.util.Map" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EResource" instanceClassName="org.eclipse.emf.ecore.resource.Resource" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EResourceSet" instanceClassName="org.eclipse.emf.ecore.resource.ResourceSet" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EShort" instanceClassName="short"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EShortObject" instanceClassName="java.lang.Short"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EString" instanceClassName="java.lang.String"/>
  <eClassifiers xsi:type="ecore:EDataType" name="ETreeIterator" instanceClassName="org.eclipse.emf.common.util.TreeIterator" serializable="false"/>
  <eClassifiers xsi:type="ecore:EDataType" name="EInvocationTargetException" instanceClassName="java.lang.reflect.InvocationTargetException" serializable="false"/>
</ecore:EPackage>
"##;
