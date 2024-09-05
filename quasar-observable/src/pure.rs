use super::*;

/// Marker trait to add a pure observable instance to a type. The observable
/// will always contain `self`.
pub trait PureObservable: Send + Clone {}

impl PureObservable for bool {}

impl PureObservable for i8 {}
impl PureObservable for i16 {}
impl PureObservable for i32 {}
impl PureObservable for i64 {}
impl PureObservable for i128 {}
impl PureObservable for isize {}

impl PureObservable for u8 {}
impl PureObservable for u16 {}
impl PureObservable for u32 {}
impl PureObservable for u64 {}
impl PureObservable for u128 {}
impl PureObservable for usize {}

impl<A, B> PureObservable for (A, B)
where
    A: PureObservable,
    B: PureObservable,
{
}

impl<A, B, C> PureObservable for (A, B, C)
where
    A: PureObservable,
    B: PureObservable,
    C: PureObservable,
{
}

impl<T> Observable for T
where
    T: PureObservable + 'static,
{
    type T = T;
    type E = !;
    type W = !;
    type U = !;

    fn attach_return<P>(self, mut observer: P) -> Attached<Self>
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
    {
        let _ = observer.set_live(Some(Ok(self.clone())));
        Attached::new(|| self)
    }

    fn attach_return_box(
        self: Box<Self>,
        mut observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<ObservableBox<Self::T, Self::E, Self::W, Self::U>> {
        let _ = (*observer).set_live(Some(Ok((*self).clone())));
        Attached::new(|| ObservableBox::new(*self))
    }

    fn attach<P>(self, mut observer: P) -> Attached<()>
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
    {
        let _ = observer.set_live(Some(Ok(self)));
        Attached::new(|| ())
    }

    fn attach_box(
        self: Box<Self>,
        mut observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<()> {
        let _ = (*observer).set_live(Some(Ok(*self)));
        Attached::new(|| ())
    }

    fn share(self) -> impl SharedObservable<T = Self::T, E = Self::E, W = Self::W, U = Self::U>
    where
        Self: Send + Sized,
        Self::T: Clone,
        Self::E: Clone,
        Self::W: Clone,
    {
        self
    }

    fn retrieve(self) -> impl Future<Output = Result<Self::T, Self::E>> + Send
    where
        Self: Sized,
    {
        std::future::ready(Ok(self))
    }
}

impl<T> SharedObservable for T
where
    T: PureObservable + Send + Clone + 'static,
{
    fn attach_return_shared_box(
        self: Box<Self>,
        mut observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<SharedObservableBox<Self::T, Self::E, Self::W, Self::U>> {
        let _ = (*observer).set_live(Some(Ok((*self).clone())));
        Attached::new(|| SharedObservableBox::new(*self))
    }

    fn clone_box(&self) -> SharedObservableBox<Self::T, Self::E, Self::W, Self::U> {
        SharedObservableBox::new(self.clone())
    }
}
