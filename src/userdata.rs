use std::cell::{Ref, RefCell, RefMut};

use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::lua::Lua;
use crate::table::Table;
use crate::types::LuaRef;
use crate::util::{assert_stack, get_userdata, StackGuard};
use crate::value::{FromLua, FromLuaMulti, ToLua, ToLuaMulti};

/// Kinds of metamethods that can be overridden.
///
/// Currently, this mechanism does not allow overriding the `__gc` metamethod, since there is
/// generally no need to do so: [`UserData`] implementors can instead just implement `Drop`.
///
/// [`UserData`]: trait.UserData.html
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum MetaMethod {
    /// The `+` operator.
    Add,
    /// The `-` operator.
    Sub,
    /// The `*` operator.
    Mul,
    /// The `/` operator.
    Div,
    /// The `%` operator.
    Mod,
    /// The `^` operator.
    Pow,
    /// The unary minus (`-`) operator.
    Unm,
    #[cfg(feature = "lua53")]
    /// The floor division (//) operator.
    IDiv,
    #[cfg(feature = "lua53")]
    /// The bitwise AND (&) operator.
    BAnd,
    #[cfg(feature = "lua53")]
    /// The bitwise OR (|) operator.
    BOr,
    #[cfg(feature = "lua53")]
    /// The bitwise XOR (binary ~) operator.
    BXor,
    #[cfg(feature = "lua53")]
    /// The bitwise NOT (unary ~) operator.
    BNot,
    #[cfg(feature = "lua53")]
    /// The bitwise left shift (<<) operator.
    Shl,
    #[cfg(feature = "lua53")]
    /// The bitwise right shift (>>) operator.
    Shr,
    /// The string concatenation operator `..`.
    Concat,
    /// The length operator `#`.
    Len,
    /// The `==` operator.
    Eq,
    /// The `<` operator.
    Lt,
    /// The `<=` operator.
    Le,
    /// Index access `obj[key]`.
    Index,
    /// Index write access `obj[key] = value`.
    NewIndex,
    /// The call "operator" `obj(arg1, args2, ...)`.
    Call,
    /// The `__tostring` metamethod.
    ///
    /// This is not an operator, but will be called by methods such as `tostring` and `print`.
    ToString,
    #[cfg(any(feature = "lua53", feature = "lua52"))]
    /// The `__pairs` metamethod.
    ///
    /// This is not an operator, but it will be called by the built-in `pairs` function.
    Pairs,
    #[cfg(any(feature = "lua53", feature = "lua52"))]
    /// The `__ipairs` metamethod.
    ///
    /// This is not an operator, but it will be called by the built-in `ipairs` function.
    IPairs,
}

impl MetaMethod {
    pub(crate) fn name(self) -> &'static [u8] {
        match self {
            MetaMethod::Add => b"__add",
            MetaMethod::Sub => b"__sub",
            MetaMethod::Mul => b"__mul",
            MetaMethod::Div => b"__div",
            MetaMethod::Mod => b"__mod",
            MetaMethod::Pow => b"__pow",
            MetaMethod::Unm => b"__unm",
            #[cfg(feature = "lua53")]
            MetaMethod::IDiv => b"__idiv",
            #[cfg(feature = "lua53")]
            MetaMethod::BAnd => b"__band",
            #[cfg(feature = "lua53")]
            MetaMethod::BOr => b"__bor",
            #[cfg(feature = "lua53")]
            MetaMethod::BXor => b"__bxor",
            #[cfg(feature = "lua53")]
            MetaMethod::BNot => b"__bnot",
            #[cfg(feature = "lua53")]
            MetaMethod::Shl => b"__shl",
            #[cfg(feature = "lua53")]
            MetaMethod::Shr => b"__shr",
            MetaMethod::Concat => b"__concat",
            MetaMethod::Len => b"__len",
            MetaMethod::Eq => b"__eq",
            MetaMethod::Lt => b"__lt",
            MetaMethod::Le => b"__le",
            MetaMethod::Index => b"__index",
            MetaMethod::NewIndex => b"__newindex",
            MetaMethod::Call => b"__call",
            MetaMethod::ToString => b"__tostring",
            #[cfg(any(feature = "lua53", feature = "lua52"))]
            MetaMethod::Pairs => b"__pairs",
            #[cfg(any(feature = "lua53", feature = "lua52"))]
            MetaMethod::IPairs => b"__ipairs",
        }
    }
}

/// Method registry for [`UserData`] implementors.
///
/// [`UserData`]: trait.UserData.html
pub trait UserDataMethods<'lua, T: UserData> {
    /// Add a method which accepts a `&T` as the first parameter.
    ///
    /// Regular methods are implemented by overriding the `__index` metamethod and returning the
    /// accessed method. This allows them to be used with the expected `userdata:method()` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular method is found.
    fn add_method<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + Fn(&'lua Lua, &T, A) -> Result<R>;

    /// Add a regular method which accepts a `&mut T` as the first parameter.
    ///
    /// Refer to [`add_method`] for more information about the implementation.
    ///
    /// [`add_method`]: #method.add_method
    fn add_method_mut<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + FnMut(&'lua Lua, &mut T, A) -> Result<R>;

    /// Add a regular method as a function which accepts generic arguments, the first argument will
    /// be a `UserData` of type T if the method is called with Lua method syntax:
    /// `my_userdata:my_method(arg1, arg2)`, or it is passed in as the first argument:
    /// `my_userdata.my_method(my_userdata, arg1, arg2)`.
    ///
    /// Prefer to use [`add_method`] or [`add_method_mut`] as they are easier to use.
    ///
    /// [`add_method`]: #method.add_method
    /// [`add_method_mut`]: #method.add_method_mut
    fn add_function<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + Fn(&'lua Lua, A) -> Result<R>;

    /// Add a regular method as a mutable function which accepts generic arguments.
    ///
    /// This is a version of [`add_function`] that accepts a FnMut argument.
    ///
    /// [`add_function`]: #method.add_function
    fn add_function_mut<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + FnMut(&'lua Lua, A) -> Result<R>;

    /// Add a metamethod which accepts a `&T` as the first parameter.
    ///
    /// # Note
    ///
    /// This can cause an error with certain binary metamethods that can trigger if only the right
    /// side has a metatable. To prevent this, use [`add_meta_function`].
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    fn add_meta_method<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + Fn(&'lua Lua, &T, A) -> Result<R>;

    /// Add a metamethod as a function which accepts a `&mut T` as the first parameter.
    ///
    /// # Note
    ///
    /// This can cause an error with certain binary metamethods that can trigger if only the right
    /// side has a metatable. To prevent this, use [`add_meta_function`].
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    fn add_meta_method_mut<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + FnMut(&'lua Lua, &mut T, A) -> Result<R>;

    /// Add a metamethod which accepts generic arguments.
    ///
    /// Metamethods for binary operators can be triggered if either the left or right argument to
    /// the binary operator has a metatable, so the first argument here is not necessarily a
    /// userdata of type `T`.
    fn add_meta_function<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + Fn(&'lua Lua, A) -> Result<R>;

    /// Add a metamethod as a mutable function which accepts generic arguments.
    ///
    /// This is a version of [`add_meta_function`] that accepts a FnMut argument.
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    fn add_meta_function_mut<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + FnMut(&'lua Lua, A) -> Result<R>;
}

/// Trait for custom userdata types.
///
/// By implementing this trait, a struct becomes eligible for use inside Lua code. Implementations
/// of [`ToLua`] and [`FromLua`] are automatically provided.
///
/// # Examples
///
/// ```
/// # use mlua::{Lua, Result, UserData};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// struct MyUserData(i32);
///
/// impl UserData for MyUserData {}
///
/// // `MyUserData` now implements `ToLua`:
/// lua.globals().set("myobject", MyUserData(123))?;
///
/// lua.load("assert(type(myobject) == 'userdata')").exec()?;
/// # Ok(())
/// # }
/// ```
///
/// Custom methods and operators can be provided by implementing `add_methods` (refer to
/// [`UserDataMethods`] for more information):
///
/// ```
/// # use mlua::{Lua, MetaMethod, Result, UserData, UserDataMethods};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// struct MyUserData(i32);
///
/// impl UserData for MyUserData {
///     fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
///         methods.add_method("get", |_, this, _: ()| {
///             Ok(this.0)
///         });
///
///         methods.add_method_mut("add", |_, this, value: i32| {
///             this.0 += value;
///             Ok(())
///         });
///
///         methods.add_meta_method(MetaMethod::Add, |_, this, value: i32| {
///             Ok(this.0 + value)
///         });
///     }
/// }
///
/// lua.globals().set("myobject", MyUserData(123))?;
///
/// lua.load(r#"
///     assert(myobject:get() == 123)
///     myobject:add(7)
///     assert(myobject:get() == 130)
///     assert(myobject + 10 == 140)
/// "#).exec()?;
/// # Ok(())
/// # }
/// ```
///
/// [`ToLua`]: trait.ToLua.html
/// [`FromLua`]: trait.FromLua.html
/// [`UserDataMethods`]: trait.UserDataMethods.html
pub trait UserData: Sized {
    /// Adds custom methods and operators specific to this userdata.
    fn add_methods<'lua, T: UserDataMethods<'lua, Self>>(_methods: &mut T) {}
}

/// Handle to an internal Lua userdata for any type that implements [`UserData`].
///
/// Similar to `std::any::Any`, this provides an interface for dynamic type checking via the [`is`]
/// and [`borrow`] methods.
///
/// Internally, instances are stored in a `RefCell`, to best match the mutable semantics of the Lua
/// language.
///
/// # Note
///
/// This API should only be used when necessary. Implementing [`UserData`] already allows defining
/// methods which check the type and acquire a borrow behind the scenes.
///
/// [`UserData`]: trait.UserData.html
/// [`is`]: #method.is
/// [`borrow`]: #method.borrow
#[derive(Clone, Debug)]
pub struct AnyUserData<'lua>(pub(crate) LuaRef<'lua>);

impl<'lua> AnyUserData<'lua> {
    /// Checks whether the type of this userdata is `T`.
    pub fn is<T: 'static + UserData>(&self) -> bool {
        match self.inspect(|_: &RefCell<T>| Ok(())) {
            Ok(()) => true,
            Err(Error::UserDataTypeMismatch) => false,
            Err(_) => unreachable!(),
        }
    }

    /// Borrow this userdata immutably if it is of type `T`.
    ///
    /// # Errors
    ///
    /// Returns a `UserDataBorrowError` if the userdata is already mutably borrowed. Returns a
    /// `UserDataTypeMismatch` if the userdata is not of type `T`.
    pub fn borrow<T: 'static + UserData>(&self) -> Result<Ref<T>> {
        self.inspect(|cell| Ok(cell.try_borrow().map_err(|_| Error::UserDataBorrowError)?))
    }

    /// Borrow this userdata mutably if it is of type `T`.
    ///
    /// # Errors
    ///
    /// Returns a `UserDataBorrowMutError` if the userdata is already borrowed. Returns a
    /// `UserDataTypeMismatch` if the userdata is not of type `T`.
    pub fn borrow_mut<T: 'static + UserData>(&self) -> Result<RefMut<T>> {
        self.inspect(|cell| {
            Ok(cell
                .try_borrow_mut()
                .map_err(|_| Error::UserDataBorrowMutError)?)
        })
    }

    /// Sets an associated value to this `AnyUserData`.
    ///
    /// The value may be any Lua value whatsoever, and can be retrieved with [`get_user_value`].
    ///
    /// [`get_user_value`]: #method.get_user_value
    pub fn set_user_value<V: ToLua<'lua>>(&self, v: V) -> Result<()> {
        let lua = self.0.lua;
        #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
        let v = {
            // Lua 5.2/5.1 allows to store only table. Then we will wrap the value.
            let t = lua.create_table()?;
            t.raw_set(1, v)?;
            crate::Value::Table(t)
        };
        #[cfg(feature = "lua53")]
        let v = v.to_lua(lua)?;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 2);
            lua.push_ref(&self.0);
            lua.push_value(v)?;
            ffi::lua_setuservalue(lua.state, -2);
            Ok(())
        }
    }

    /// Returns an associated value set by [`set_user_value`].
    ///
    /// [`set_user_value`]: #method.set_user_value
    pub fn get_user_value<V: FromLua<'lua>>(&self) -> Result<V> {
        let lua = self.0.lua;
        let res = unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);
            lua.push_ref(&self.0);
            ffi::lua_getuservalue(lua.state, -1);
            lua.pop_value()
        };
        #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
        return crate::Table::from_lua(res, lua)?.get(1);
        #[cfg(feature = "lua53")]
        V::from_lua(res, lua)
    }

    fn get_metatable(&self) -> Result<Table<'lua>> {
        unsafe {
            let lua = self.0.lua;
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);

            lua.push_ref(&self.0);

            if ffi::lua_getmetatable(lua.state, -1) == 0 {
                return Err(Error::UserDataTypeMismatch);
            }

            Ok(Table(lua.pop_ref()))
        }
    }

    pub(crate) fn equals<T: AsRef<Self>>(&self, other: T) -> Result<bool> {
        let other = other.as_ref();
        if self == other {
            return Ok(true);
        }

        let mt = self.get_metatable()?;
        if mt != other.get_metatable()? {
            return Ok(false);
        }

        if mt.contains_key("__eq")? {
            return mt
                .get::<_, Function>("__eq")?
                .call((self.clone(), other.clone()));
        }

        Ok(false)
    }

    fn inspect<'a, T, R, F>(&'a self, func: F) -> Result<R>
    where
        T: 'static + UserData,
        F: FnOnce(&'a RefCell<T>) -> Result<R>,
    {
        unsafe {
            let lua = self.0.lua;
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);

            lua.push_ref(&self.0);

            if ffi::lua_getmetatable(lua.state, -1) == 0 {
                Err(Error::UserDataTypeMismatch)
            } else {
                ffi::lua_rawgeti(
                    lua.state,
                    ffi::LUA_REGISTRYINDEX,
                    lua.userdata_metatable::<T>()? as ffi::lua_Integer,
                );

                if ffi::lua_rawequal(lua.state, -1, -2) == 0 {
                    Err(Error::UserDataTypeMismatch)
                } else {
                    func(&*get_userdata::<RefCell<T>>(lua.state, -3))
                }
            }
        }
    }
}

impl<'lua> PartialEq for AnyUserData<'lua> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<'lua> AsRef<AnyUserData<'lua>> for AnyUserData<'lua> {
    #[inline]
    fn as_ref(&self) -> &Self {
        self
    }
}
