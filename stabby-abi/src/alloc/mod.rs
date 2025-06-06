//
// Copyright (c) 2023 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   Pierre Avital, <pierre.avital@me.com>
//

#![allow(deprecated)]
use core::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull, sync::atomic::AtomicUsize};

use self::vec::ptr_diff;

/// Allocators provided by `stabby`
pub mod allocators;

/// A generic allocation error.
#[crate::stabby]
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct AllocationError();
impl core::fmt::Display for AllocationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AllocationError")
    }
}
#[cfg(feature = "std")]
impl std::error::Error for AllocationError {}

/// [`alloc::boxed`](https://doc.rust-lang.org/stable/alloc/boxed/), but ABI-stable.
pub mod boxed;
/// Allocated collections, including immutable ones.
pub mod collections;
/// A vector that stores a single element on the stack until allocation is necessary.
pub mod single_or_vec;
/// [`alloc::string`](https://doc.rust-lang.org/stable/alloc/string/), but ABI-stable
pub mod string;
/// [`alloc::sync`](https://doc.rust-lang.org/stable/alloc/sync/), but ABI-stable
pub mod sync;
/// [`alloc::vec`](https://doc.rust-lang.org/stable/alloc/vec/), but ABI-stable
pub mod vec;

/// The default allocator, depending on which of the following is available:
/// - RustAlloc: Rust's `GlobalAlloc`, through a vtable that ensures FFI-safety.
/// - LibcAlloc: libc::malloc, which is 0-sized.
/// - None. I _am_ working on getting a 0-dependy allocator working, but you should probably go with `feature = "alloc-rs"` anyway.
///
/// You can also use the `stabby_default_alloc` cfg to override the default allocator regardless of feature flags.
pub type DefaultAllocator = allocators::DefaultAllocator;

#[crate::stabby]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// ABI-stable equivalent of std::mem::Layout
pub struct Layout {
    /// The expected size of the allocation.
    pub size: usize,
    /// The expected alignment of the allocation.
    pub align: usize,
}
impl Layout {
    /// Returns the [`Layout`] corresponding to `T`
    pub const fn of<T: Sized>() -> Self {
        Layout {
            size: core::mem::size_of::<T>(),
            align: core::mem::align_of::<T>(),
        }
    }
    /// Returns the [`Layout`] corresponding to `[T; n]`.
    ///
    /// Note that while this ensures that even if `T`'s size is not a multiple of its alignment,
    /// the layout will have sufficient memory to store `n` of `T` in an aligned fashion.
    pub const fn array<T: Sized>(n: usize) -> Self {
        let Self { size, align } = Self::of::<T>();
        Layout {
            size: size * n,
            align,
        }
    }
    /// Concatenates a layout to `self`, ensuring that alignment padding is taken into account.
    pub const fn concat(mut self, other: Self) -> Self {
        self.size += other.size;
        self.realign(if self.align < other.align {
            other.align
        } else {
            self.align
        })
    }
    /// Returns the first pointer where `output >= ptr` such that `output % self.align == 0`.
    #[inline]
    pub fn next_matching<T>(self, ptr: *mut T) -> *mut T {
        fn next_matching(align: usize, ptr: *mut u8) -> *mut u8 {
            unsafe { ptr.add(ptr.align_offset(align)) }
        }
        next_matching(self.align, ptr.cast()).cast()
    }
    /// Changes the alignment of the layout, adding padding if necessary.
    pub const fn realign(mut self, new_align: usize) -> Self {
        self.align = new_align;
        self.size = self.size
            + (new_align - (self.size % new_align)) * (((self.size % new_align) != 0) as usize);
        self
    }
}

/// An interface to an allocator.
///
/// Note that `stabby` often stores allocators inside allocations they made, so allocators that cannot allocate
/// more than their size on stack will systematically fail to construct common stabby types.
///
/// Since the allocator may be moved, it must also be safe to do so, including after it has performed allocations.
pub trait IAlloc: Unpin {
    /// Allocates at least as much memory as requested by layout, ensuring the requested alignment is respected.
    ///
    /// If the requested size is 0, or allocation failed, then a null pointer is returned.
    fn alloc(&mut self, layout: Layout) -> *mut ();
    /// Frees the allocation
    ///
    /// # Safety
    /// `ptr` MUST have been allocated through a succesful call to `Self::alloc` or `Self::realloc` with the same instance of `Self`
    unsafe fn free(&mut self, ptr: *mut ());
    /// Reallocates `ptr`, ensuring that it has enough memory for the newly requested layout.
    ///
    /// If the requested size is 0, or allocation failed, then a null pointer is returned, and `ptr` is not freed.
    ///
    /// # Safety
    /// `ptr` MUST have been allocated through a succesful call to `Self::alloc` with the same instance of `Self`
    unsafe fn realloc(&mut self, ptr: *mut (), prev_layout: Layout, new_size: usize) -> *mut () {
        let ret = self.alloc(Layout {
            size: new_size,
            align: prev_layout.align,
        });
        if !ret.is_null() {
            unsafe {
                core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), ret.cast(), prev_layout.size);
                self.free(ptr);
            }
        }
        ret
    }
}

/// An ABI stable equivalent to [`IAlloc`].
#[crate::stabby]
#[deprecated = "Stabby doesn't actually use this trait due to conflicts."]
pub trait IStableAlloc: Unpin {
    /// Allocates at least as much memory as requested by layout, ensuring the requested alignment is respected.
    ///
    /// If the requested size is 0, or allocation failed, then a null pointer is returned.
    extern "C" fn alloc(&mut self, layout: Layout) -> *mut ();
    /// Frees the allocation
    ///
    /// # Safety
    /// `ptr` MUST have been allocated through a succesful call to `Self::alloc` or `Self::realloc` with the same instance of `Self`
    extern "C" fn free(&mut self, ptr: *mut ());
    /// Reallocates `ptr`, ensuring that it has enough memory for the newly requested layout.
    ///
    /// If the requested size is 0, or allocation failed, then a null pointer is returned, and `ptr` is not freed.
    ///
    /// # Safety
    /// `ptr` MUST have been allocated through a succesful call to `Self::alloc` with the same instance of `Self`
    extern "C" fn realloc(
        &mut self,
        ptr: *mut (),
        prev_layout: Layout,
        new_size: usize,
    ) -> *mut () {
        let ret = self.alloc(Layout {
            size: new_size,
            align: prev_layout.align,
        });
        if !ret.is_null() {
            unsafe {
                core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), ret.cast(), prev_layout.size);
                self.free(ptr);
            }
        }
        ret
    }
}
#[allow(clippy::not_unsafe_ptr_arg_deref)]
impl<T: IAlloc> IStableAlloc for T {
    extern "C" fn alloc(&mut self, layout: Layout) -> *mut () {
        IAlloc::alloc(self, layout)
    }
    extern "C" fn free(&mut self, ptr: *mut ()) {
        unsafe { IAlloc::free(self, ptr) }
    }
    extern "C" fn realloc(
        &mut self,
        ptr: *mut (),
        prev_layout: Layout,
        new_size: usize,
    ) -> *mut () {
        unsafe { IAlloc::realloc(self, ptr, prev_layout, new_size) }
    }
}

impl<T: IStableAllocDynMut<crate::vtable::H> + Unpin> IAlloc for T {
    fn alloc(&mut self, layout: Layout) -> *mut () {
        IStableAllocDynMut::alloc(self, layout)
    }
    unsafe fn free(&mut self, ptr: *mut ()) {
        IStableAllocDynMut::free(self, ptr)
    }
    unsafe fn realloc(&mut self, ptr: *mut (), prev_layout: Layout, new_size: usize) -> *mut () {
        IStableAllocDynMut::realloc(self, ptr, prev_layout, new_size)
    }
}
impl IAlloc for core::convert::Infallible {
    fn alloc(&mut self, _layout: Layout) -> *mut () {
        unreachable!()
    }
    unsafe fn free(&mut self, _ptr: *mut ()) {
        unreachable!()
    }
}

/// The prefix common to all allocations in [`stabby::alloc`](crate::alloc).
///
/// This allows reuse of allocations when converting between container types.
#[crate::stabby]
pub struct AllocPrefix<Alloc> {
    /// The strong count for reference counted types.
    pub strong: core::sync::atomic::AtomicUsize,
    /// The weak count for reference counted types.
    pub weak: core::sync::atomic::AtomicUsize,
    /// A slot to store a vector's capacity when it's turned into a boxed/arced slice.
    pub capacity: core::sync::atomic::AtomicUsize,
    /// The origin of the prefix
    pub origin: NonNull<()>,
    /// A slot for the allocator.
    pub alloc: core::mem::MaybeUninit<Alloc>,
}
impl<Alloc> AllocPrefix<Alloc> {
    /// The offset between the prefix and a field of type `T`.
    pub const fn skip_to<T>() -> usize {
        let mut size = core::mem::size_of::<Self>();
        let align = core::mem::align_of::<T>();
        let sizemodalign = size % align;
        if sizemodalign != 0 {
            size += align;
            size -= sizemodalign;
        }
        size
    }
}

/// A non-null pointer guaranteed to be preceded by a valid
/// [`AllocPrefix`] unless the pointer is dangling.
///
/// This means that unless `T` is a ZST, the pointer is guaranteed to be aligned to the maximum of `T`'s alignment and the alignment of the prefix, which itself is ptr-size aligned.
#[crate::stabby]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AllocPtr<T, Alloc> {
    /// The pointer to the data.
    pub ptr: NonNull<T>,
    /// Remember the allocator's type.
    pub marker: PhantomData<Alloc>,
}
impl<T, Alloc> Copy for AllocPtr<T, Alloc> {}
impl<T, Alloc> Clone for AllocPtr<T, Alloc> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, Alloc> core::ops::Deref for AllocPtr<T, Alloc> {
    type Target = NonNull<T>;
    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}
impl<T, Alloc> core::ops::DerefMut for AllocPtr<T, Alloc> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ptr
    }
}
impl<T, Alloc> AllocPtr<MaybeUninit<T>, Alloc> {
    /// Assumes the internals of the pointer have been initialized.
    /// # Safety
    /// The internals of the pointer must have been initialized.
    pub const unsafe fn assume_init(self) -> AllocPtr<T, Alloc> {
        unsafe { core::mem::transmute::<Self, AllocPtr<T, Alloc>>(self) }
    }
}
impl<T, Alloc> AllocPtr<T, Alloc> {
    /// Constructs a dangling pointer.
    pub const fn dangling() -> Self {
        Self {
            ptr: NonNull::dangling(),
            marker: PhantomData,
        }
    }
    /// Casts an allocated pointer.
    pub const fn cast<U>(self) -> AllocPtr<U, Alloc> {
        AllocPtr {
            ptr: self.ptr.cast(),
            marker: PhantomData,
        }
    }
    ///The pointer to the prefix for this allocation
    const fn prefix_ptr(&self) -> NonNull<AllocPrefix<Alloc>> {
        unsafe { NonNull::new_unchecked(self.ptr.as_ptr().cast::<AllocPrefix<Alloc>>().sub(1)) }
    }
    /// A reference to the prefix for this allocation.
    /// # Safety
    /// `self` must not be dangling, and have been properly allocated, using [`Self::alloc`] or [`Self::realloc`] for example.
    #[rustversion::attr(since(1.73), const)]
    pub unsafe fn prefix(&self) -> &AllocPrefix<Alloc> {
        unsafe { self.prefix_ptr().as_ref() }
    }
    /// A mutable reference to the prefix for this allocation.
    /// # Safety
    /// `self` must not be dangling, and have been properly allocated, using [`Self::alloc`] or [`Self::realloc`] for example.
    /// Since this type is [`Copy`], the `&mut self` is not a sufficient guarantee of uniqueness.
    #[rustversion::attr(since(1.86), const)]
    pub unsafe fn prefix_mut(&mut self) -> &mut AllocPrefix<Alloc> {
        unsafe { self.prefix_ptr().as_mut() }
    }
    /// Returns mutable access to the prefix and the data.
    /// # Safety
    /// `self` must not be dangling, and have been properly allocated, using [`Self::alloc`] or [`Self::realloc`] for example.
    #[rustversion::attr(since(1.86), const)]
    pub unsafe fn split_mut(&mut self) -> (&mut AllocPrefix<Alloc>, &mut T) {
        let prefix = self.prefix_ptr().as_mut();
        let data = self.ptr.as_mut();
        (prefix, data)
    }
    /// Initializes any given pointer:
    /// - The returned pointer is guaranteed to be correctly aligned for `T`
    /// - It is guaranteed to preceded without padding by an `AllocPrefix<Alloc>`
    /// # Safety
    /// `ptr` MUST be word-aligned, and MUST be valid for writes for at least the size of
    /// `#[repr(C)] struct { prefix: AllocPrefix<Alloc>, data: [T; capacity] }`
    pub unsafe fn init(ptr: NonNull<()>, capacity: usize) -> Self {
        let shifted_for_prefix = ptr
            .as_ptr()
            .cast::<AllocPrefix<Alloc>>()
            .add(1)
            .cast::<u8>();
        let inited = shifted_for_prefix
            .cast::<u8>()
            .add(shifted_for_prefix.align_offset(core::mem::align_of::<T>()))
            .cast::<T>();
        let this: Self = AllocPtr {
            ptr: NonNull::new_unchecked(inited),
            marker: core::marker::PhantomData,
        };
        this.prefix_ptr().as_ptr().write(AllocPrefix {
            strong: AtomicUsize::new(1),
            weak: AtomicUsize::new(1),
            capacity: AtomicUsize::new(capacity),
            origin: ptr,
            alloc: core::mem::MaybeUninit::uninit(),
        });
        this
    }
}
impl<T, Alloc: IAlloc> AllocPtr<T, Alloc> {
    /// Allocates a pointer to a single element of `T`, prefixed by an [`AllocPrefix`]
    pub fn alloc(alloc: &mut Alloc) -> Option<Self> {
        Self::alloc_array(alloc, 1)
    }
    /// Allocates a pointer to an array of `capacity` `T`, prefixed by an [`AllocPrefix`]
    pub fn alloc_array(alloc: &mut Alloc, capacity: usize) -> Option<Self> {
        let mut layout = Layout::of::<AllocPrefix<Alloc>>().concat(Layout::array::<T>(capacity));
        layout.align = core::mem::align_of::<AllocPrefix<Alloc>>();
        let ptr = alloc.alloc(layout);
        NonNull::new(ptr).map(|ptr| unsafe { Self::init(ptr, capacity) })
    }
    /// Reallocates a pointer to an array of `capacity` `T`, prefixed by an [`AllocPrefix`].
    ///
    /// In case of failure of the allocator, this will return `None` and `self` will not have been freed.
    ///
    /// # Safety
    /// `self` must not be dangling
    pub unsafe fn realloc(
        self,
        alloc: &mut Alloc,
        prev_capacity: usize,
        new_capacity: usize,
    ) -> Option<Self> {
        let mut layout =
            Layout::of::<AllocPrefix<Alloc>>().concat(Layout::array::<T>(prev_capacity));
        layout.align = core::mem::align_of::<AllocPrefix<Alloc>>();
        let ptr = alloc.realloc(
            self.prefix_ptr().cast().as_ptr(),
            layout,
            Layout::of::<AllocPrefix<Alloc>>()
                .concat(Layout::array::<T>(new_capacity))
                .size,
        );
        NonNull::new(ptr).map(|ptr| unsafe { Self::init(ptr, new_capacity) })
    }
    /// Reallocates a pointer to an array of `capacity` `T`, prefixed by an [`AllocPrefix`]
    /// # Safety
    /// `self` must not be dangling, and is freed after this returns.
    pub unsafe fn free(self, alloc: &mut Alloc) {
        alloc.free(self.prefix().origin.as_ptr())
    }
}

/// A helper to work with allocated slices.
#[crate::stabby]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AllocSlice<T, Alloc> {
    /// The start of the slice.
    pub start: AllocPtr<T, Alloc>,
    /// The end of the slice (exclusive).
    pub end: NonNull<T>,
}
impl<T, Alloc> AllocSlice<T, Alloc> {
    /// Returns the number of elements in the slice.
    pub const fn len(&self) -> usize {
        ptr_diff(self.end, self.start.ptr)
    }
    /// Returns `true` if the slice is empty.
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Returns this slice.
    /// # Safety
    /// `self` must be valid.
    pub const unsafe fn as_slice(&self) -> &[T] {
        core::slice::from_raw_parts(self.start.ptr.as_ptr(), ptr_diff(self.end, self.start.ptr))
    }
}
impl<T, Alloc> Copy for AllocSlice<T, Alloc> {}
impl<T, Alloc> Clone for AllocSlice<T, Alloc> {
    fn clone(&self) -> Self {
        *self
    }
}
