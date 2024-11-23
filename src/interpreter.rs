use anyhow::{anyhow, bail, ensure, Result};

use crate::binary_grammar::{Expression, RefType, ValueType};
use crate::execution_grammar::{
    Entry, ExternalImport, Frame, FunctionInstance, Label, ModuleInstance, Ref, Stack, Value,
};
use crate::Parser;
use crate::Store;

#[derive(Debug)]
pub struct Interpreter<'a> {
    module_instances: Vec<ModuleInstance<'a>>,
    stack: Stack<'a>,
    store: Store<'a>,
    globals: Vec<Value>,
}

impl<'a> Interpreter<'a> {
    pub fn new(
        module_data: &'a [u8],
        imports: Vec<ExternalImport>,
        initial_global_values: Vec<Value>,
        element_segment_refs: Vec<Vec<Ref>>,
    ) -> Result<Self> {
        let module = Parser::new(module_data).parse_module()?;

        ensure!(
            module.imports.len() == imports.len(),
            "The number of module imports is not equal to the number of provided external values."
        );

        let mut store = Store::new();

        let module_instance =
            store.allocate_module(module, imports, initial_global_values, element_segment_refs)?;

        // store the auxiliary frame
        Ok(Self {
            module_instances: vec![module_instance],
            stack: Stack::new(),
            store,
            globals: vec![],
        })
    }

    pub fn invoke(&mut self, function_addr: usize, args: Vec<Value>) -> Result<()> {
        let function_instance = self.store.functions.get(function_addr).ok_or(anyhow!(
            "Function address: {} does not exist in store.",
            function_addr
        ))?;

        match function_instance {
            FunctionInstance::Local { function_type, .. } => {
                let f_num_args = function_type.0 .0.len();
                ensure!(f_num_args == args.len(), "Length of provided argument values is different from the number of expected arguments.");

                // validate that every value correspond with value type
                for (value_type, value) in function_type.0 .0.iter().zip(&args) {
                    match (value, value_type) {
                        (Value::I32(_), ValueType::I32)
                        | (Value::I64(_), ValueType::I64)
                        | (Value::F32(_), ValueType::F32)
                        | (Value::F64(_), ValueType::F64)
                        | (Value::V128(_), ValueType::V128)
                        | (Value::Ref(Ref::FunctionAddr(_)), ValueType::Ref(RefType::FuncRef))
                        | (Value::Ref(Ref::RefExtern(_)), ValueType::Ref(RefType::ExternRef)) => {}
                        _ => bail!("Value does not correspond with value type."),
                    }
                }

                self.stack.push(Entry::Activation(Frame::default()));

                for arg in args {
                    self.stack.push(Entry::Value(arg));
                }
            }
            _ => todo!("Handle host function instance"),
        }

        Ok(())
    }

    fn invoke_expression(&mut self, expression: Vec<Expression>, frame: Frame<'a>) -> Result<()> {
        Ok(())
    }

    fn invoke_function_call(&mut self, function_addr: usize) -> Result<()> {
        let function_instance = &self.store.functions[function_addr];

        match function_instance {
            FunctionInstance::Local {
                function_type,
                code,
                module,
            } => {
                let num_args = function_type.0 .0.len();
                ensure!(self.stack.len() >= num_args, "At least {num_args} values must be on top of the stack".);

                let mut locals = self.stack.pop_n(num_args)?;

                code.locals
                    .iter()
                    .for_each(|&local| locals.push(Entry::Value(Value::default(local.value_type))));

                let num_ret_args = function_type.1 .0.len();

                let f = Frame {
                    arity: num_ret_args,
                    locals,
                    module,
                };

                self.stack.push(Entry::Activation(f.clone()));

                let l = Label {
                    arity: num_ret_args,
                    continuation: code.body.clone(),
                };

                self.stack.push(Entry::Label(l));

                self.invoke_expression(code.body.clone(), f)?;

                if num_ret_args == 0 {
                    // no return
                }
            }
            _ => todo!("What does invoking a Host Function look like"),
        }

        Ok(())
    }
}
