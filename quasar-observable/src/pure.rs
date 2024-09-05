use super::*;

#[derive(Clone)]
struct Pure<T>(T)
where
    T: Clone;

impl<T> Observable for Pure<T>
where
    T: Send + Clone + 'static,
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
        todo!()
    }

    fn attach_return_box(
        self: Box<Self>,
        mut observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<ObservableBox<Self::T, Self::E, Self::W, Self::U>> {
        todo!()
    }

    fn attach<P>(self, mut observer: P) -> Attached<()>
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
    {
        let _ = observer.set_live(Some(Ok(self.0)));
        Attached::new(|| ())
    }

    fn attach_box(
        self: Box<Self>,
        mut observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<()> {
        let _ = (*observer).set_live(Some(Ok((*self).0)));
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
        std::future::ready(Ok(self.0))
    }

    fn map<F, A>(self, f: F) -> impl Observable<T = A, E = Self::E, W = Self::W, U = !>
    where
        Self: Sized,
        A: Send + Clone + 'static,
        F: Fn(Self::T) -> A + Send + Sync + 'static,
    {
        Pure(f(self.0))
    }
}

impl<T> SharedObservable for Pure<T>
where
    T: Send + Clone + 'static,
{
    fn attach_return_shared_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<SharedObservableBox<Self::T, Self::E, Self::W, Self::U>> {
        self.attach_return(observer).map(SharedObservableBox::new)
    }

    fn clone_box(&self) -> SharedObservableBox<Self::T, Self::E, Self::W, Self::U> {
        SharedObservableBox::new(self.clone())
    }
}

/// Marker trait to add a pure observable instance to a type. The observable
/// will always contain `self`.
///
/// If the type implements `Clone`, the observable will be shared.
pub trait PureObservable: Send {}

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
    T: PureObservable + Clone + 'static,
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

    fn map<F, A>(self, f: F) -> impl Observable<T = A, E = Self::E, W = Self::W, U = !>
    where
        Self: Sized,
        A: Send + Clone + 'static,
        F: Fn(Self::T) -> A + Send + Sync + 'static,
    {
        Pure(f(self))
    }
}

impl<T> SharedObservable for T
where
    T: PureObservable + Clone + 'static,
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
