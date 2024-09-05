#![feature(associated_type_defaults)]
#![feature(never_type)]
#![feature(trait_upcasting)]

use std::any::Any;
use std::boxed::Box;
use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;
use std::ops::FnMut;
use std::ops::FnOnce;
use std::sync::Arc;
use std::sync::Mutex;

pub mod pure;

pub trait Observable
where
    Self: 'static,
{
    type T: Send;
    type E: Send;
    type W: Send = !;
    type U: Update<Self::T> + Send = !;

    #[must_use]
    fn attach_return<P>(self, observer: P) -> Attached<Self>
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static;

    /// Object-safe variant of `Observable::attach_return`.
    #[must_use]
    fn attach_return_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<ObservableBox<Self::T, Self::E, Self::W, Self::U>>;

    #[must_use]
    fn attach<P>(self, observer: P) -> Attached<()>
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
    {
        self.attach_return(observer).map(|_| {})
    }

    /// Object-safe variant of `Observable::attach`.
    #[must_use]
    fn attach_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<()> {
        self.attach_return_box(observer).map(|_| {})
    }

    fn share(self) -> impl SharedObservable<T = Self::T, E = Self::E, W = Self::W, U = Self::U>
    where
        Self: Send + Sized,
        Self::T: Clone,
        Self::E: Clone,
        Self::W: Clone,
    {
        Arc::new(share::Share::new(self))
    }

    fn retrieve(self) -> impl Future<Output = Result<Self::T, Self::E>> + Send
    where
        Self: Sized,
    {
        self::retrieve::retrieve(self)
    }

    fn map<F, A>(self, f: F) -> self::map::Map<Self, F, A>
    where
        Self: Sized,
        F: Fn(Self::T) -> A + Send + Sync,
    {
        self::map::Map::new(self, f)
    }

    fn map_items<F, A>(
        self,
        f: F,
    ) -> impl Observable<
        T = <Self::T as Container>::C<A>,
        E = Self::E,
        W = Self::W,
        U = <Self::U as ContainerUpdate<Self::T>>::U<A>,
    >
    where
        Self: Sized,
        Self::T: Container,
        Self::U: ContainerUpdate<Self::T>,
        F: FnMut(<Self::T as Container>::I) -> A + Send + 'static,
        A: Send + Clone + 'static,
        <Self::T as Container>::C<A>: Send,
        <Self::U as ContainerUpdate<Self::T>>::U<A>: Send,
    {
        self::map_items::MapItems(self, f)
    }
}

pub trait SharedObservable: Observable + Send {
    /// Object-safe variant of `Observable::attach_return`.
    #[must_use]
    fn attach_return_shared_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<SharedObservableBox<Self::T, Self::E, Self::W, Self::U>>;

    // Object-safe variant of `clone()`
    fn clone_box(&self) -> SharedObservableBox<Self::T, Self::E, Self::W, Self::U>;
}

pub trait ObservableExt: Observable {
    fn eval(self) -> self::eval::Eval<Self>
    where
        Self: Sized,
    {
        self::eval::Eval(self)
    }
}

impl<O> ObservableExt for O where O: Observable {}

pub struct ObservableBox<T, E, W, U>(pub Box<dyn Observable<T = T, E = E, W = W, U = U>>);

impl<T, E, W, U> ObservableBox<T, E, W, U> {
    fn new(observable: impl Observable<T = T, E = E, W = W, U = U>) -> Self {
        ObservableBox(Box::new(observable))
    }
}

pub struct SharedObservableBox<T, E, W, U>(
    pub Box<dyn SharedObservable<T = T, E = E, W = W, U = U>>,
);

impl<T, E, W, U> SharedObservableBox<T, E, W, U> {
    fn new(observable: impl SharedObservable<T = T, E = E, W = W, U = U>) -> Self {
        SharedObservableBox(Box::new(observable))
    }
}

impl<T, E, W, U> Observable for SharedObservableBox<T, E, W, U>
where
    T: Send + 'static,
    E: Send + 'static,
    W: Send + 'static,
    U: Update<T> + Send + 'static,
{
    type T = T;
    type E = E;
    type W = W;
    type U = U;

    fn attach_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<()> {
        self.0.attach_box(observer)
    }

    fn attach_return<P>(self, observer: P) -> Attached<Self>
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
    {
        self.0.attach_return_shared_box(Box::new(observer))
    }

    fn attach_return_box(
        self: Box<Self>,
        observer: ObserverBox<T, E, W, U>,
    ) -> Attached<ObservableBox<T, E, W, U>> {
        todo!()
    }

    fn attach<P>(self, observer: P) -> Attached<()>
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
    {
        self.0.attach_box(Box::new(observer))
    }
}

impl<T, E, W, U> SharedObservable for SharedObservableBox<T, E, W, U>
where
    T: Send + 'static,
    E: Send + 'static,
    W: Send + 'static,
    U: Update<T> + Send + 'static,
{
    fn attach_return_shared_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
    ) -> Attached<SharedObservableBox<Self::T, Self::E, Self::W, Self::U>> {
        todo!()
    }

    fn clone_box(&self) -> SharedObservableBox<Self::T, Self::E, Self::W, Self::U> {
        self.0.clone_box()
    }
}

pub struct Attached<R>(Option<Box<dyn FnOnce() -> R + Send>>);

impl<R> Drop for Attached<R> {
    fn drop(&mut self) {
        if let Some(f) = std::mem::take(&mut self.0) {
            let _ = f();
        }
    }
}

impl<R> Attached<R> {
    pub fn new(f: impl FnOnce() -> R + Send + 'static) -> Self {
        Attached(Some(Box::new(f)))
    }

    pub fn detach(mut self) -> R {
        let f = std::mem::take(&mut self.0).unwrap();
        f()
    }

    pub fn map<F, A>(mut self, f: F) -> Attached<A>
    where
        R: 'static,
        F: (FnOnce(R) -> A) + Send + 'static,
    {
        let detach = std::mem::take(&mut self.0).unwrap();
        Attached::new(|| f(detach()))
    }
}

impl<F, R> From<F> for Attached<R>
where
    F: FnOnce() -> R + Send + 'static,
{
    fn from(f: F) -> Self {
        Attached::new(f)
    }
}

pub trait Update<T>: EvalUpdate<T> {
    fn apply_update(self, target: T) -> T;
    fn apply_update_mut(self, target: &mut T);
    fn merge_update(self, next: Self) -> Self;
    fn merge_update_mut(&mut self, next: Self);
}

impl<T> Update<T> for ! {
    fn apply_update(self, _target: T) -> T {
        match self {}
    }
    fn apply_update_mut(self, _target: &mut T) {
        match self {}
    }
    fn merge_update(self, _next: Self) -> Self {
        match self {}
    }
    fn merge_update_mut(&mut self, _next: Self) {
        match *self {}
    }
}

pub trait EvalUpdate<T> {
    #[must_use]
    fn attach_eval_return<O, R>(observable: O, observer: R) -> Attached<O>
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !> + 'static;

    #[must_use]
    fn attach_eval<O, R>(observable: O, observer: R) -> Attached<()>
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !> + 'static;
}

pub trait EvalUpdateMarker {}

impl<U, T> EvalUpdate<T> for U
where
    U: EvalUpdateMarker,
    T: Send + Clone,
{
    fn attach_eval_return<O, R>(observable: O, observer: R) -> Attached<O>
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !> + 'static,
    {
        observable.attach_return(self::eval::EvalObserver::<O, R>::new(observer))
    }

    fn attach_eval<O, R>(observable: O, observer: R) -> Attached<()>
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !> + 'static,
    {
        observable.attach(self::eval::EvalObserver::<O, R>::new(observer))
    }
}

impl<T> EvalUpdate<T> for ! {
    fn attach_eval_return<O, R>(observable: O, observer: R) -> Attached<O>
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !> + 'static,
    {
        observable.attach_return(observer)
    }

    fn attach_eval<O, R>(observable: O, observer: R) -> Attached<()>
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !> + 'static,
    {
        observable.attach(observer)
    }
}

pub trait Container {
    type C<X>;
    type I;

    fn map_container<F, A>(self, f: F) -> Self::C<A>
    where
        F: FnMut(Self::I) -> A;
}

// NOTE: Vec is used as a placeholder for an immutable vector
impl<I> Container for Vec<I> {
    type C<X> = Vec<X>;
    type I = I;

    fn map_container<F, A>(self, _f: F) -> Self::C<A>
    where
        F: FnMut(Self::I) -> A,
    {
        todo!()
    }
}

pub trait ContainerUpdate<T>: Update<T>
where
    T: Container,
{
    type U<X: Send + Clone>: Update<T::C<X>>;

    fn map_update<F, A>(self) -> Self::U<A>
    where
        F: FnMut(T::I) -> A,
        A: Send + Clone;
}

impl<I> ContainerUpdate<Vec<I>> for VecUpdate<I>
where
    I: Send + Clone,
{
    type U<X: Send + Clone> = VecUpdate<X>;

    fn map_update<F, A>(self) -> Self::U<A>
    where
        F: FnMut(I) -> A,
        A: Send + Clone,
    {
        todo!()
    }
}

#[derive(Clone)]
pub enum VecUpdate<I> {
    Insert(u64, I),
}

impl<I> EvalUpdateMarker for VecUpdate<I> {}

impl<I> Update<Vec<I>> for VecUpdate<I>
where
    I: Send + Clone,
{
    fn apply_update(self, _target: Vec<I>) -> Vec<I> {
        todo!()
    }

    fn apply_update_mut(self, _target: &mut Vec<I>) {
        todo!()
    }

    fn merge_update(self, _next: Self) -> Self {
        todo!()
    }

    fn merge_update_mut(&mut self, _next: Self) {
        todo!()
    }
}

pub trait Observer<T, E, W, U>: Any + Send
where
    U: Update<T>,
{
    fn set_changing(&mut self, clear_cache: bool);
    fn set_waiting(&mut self, clear_cache: bool, marker: W);
    fn set_live(&mut self, content: Option<Result<T, E>>) -> Box<dyn Future<Output = ()> + Sync>;
    fn update(&mut self, update: U) -> Box<dyn Future<Output = ()> + Sync>;
}

type ObserverBox<T, E, W, U> = Box<dyn Observer<T, E, W, U>>;

impl<T, E, W, U> Observer<T, E, W, U> for ObserverBox<T, E, W, U>
where
    T: 'static,
    E: 'static,
    W: 'static,
    U: Update<T> + 'static,
{
    fn set_changing(&mut self, clear_cache: bool) {
        (**self).set_changing(clear_cache)
    }

    fn set_waiting(&mut self, clear_cache: bool, marker: W) {
        (**self).set_waiting(clear_cache, marker)
    }

    fn set_live(&mut self, content: Option<Result<T, E>>) -> Box<dyn Future<Output = ()> + Sync> {
        (**self).set_live(content)
    }

    fn update(&mut self, update: U) -> Box<dyn Future<Output = ()> + Sync> {
        (**self).update(update)
    }
}

pub enum ObserverState<T, E, W> {
    Changing(Option<Result<T, E>>),
    Waiting(Option<Result<T, E>>, W),
    Live(Result<T, E>),
}

impl<T, E, W> Default for ObserverState<T, E, W> {
    fn default() -> Self {
        ObserverState::Changing(None)
    }
}

impl<T, E, W> ObserverState<T, E, W> {
    pub fn new() -> ObserverState<T, E, W> {
        ObserverState::Changing(None)
    }

    pub fn set_changing(self, clear_cache: bool) -> ObserverState<T, E, W> {
        if clear_cache {
            ObserverState::Changing(None)
        } else {
            match self {
                ObserverState::Changing(cache) => ObserverState::Changing(cache),
                ObserverState::Waiting(cache, _marker) => ObserverState::Changing(cache),
                ObserverState::Live(content) => ObserverState::Changing(Some(content)),
            }
        }
    }

    pub fn set_waiting(self, clear_cache: bool, marker: W) -> ObserverState<T, E, W> {
        if clear_cache {
            ObserverState::Waiting(None, marker)
        } else {
            match self {
                ObserverState::Changing(cache) => ObserverState::Waiting(cache, marker),
                ObserverState::Waiting(cache, _marker) => ObserverState::Waiting(cache, marker),
                ObserverState::Live(content) => ObserverState::Waiting(Some(content), marker),
            }
        }
    }

    pub fn set_live(self, content: Option<Result<T, E>>) -> ObserverState<T, E, W> {
        match content {
            Some(content) => ObserverState::Live(content),
            None => {
                if let Some(content) = self.cached_content() {
                    ObserverState::Live(content)
                } else {
                    // this can be the result of incorrect observable implementations
                    panic!()
                }
            }
        }
    }

    pub fn update<U>(self, update: U) -> ObserverState<T, E, W>
    where
        U: Update<T>,
    {
        if let Some(Ok(value)) = self.cached_content() {
            ObserverState::Live(Ok(update.apply_update(value)))
        } else {
            // this can be the result of incorrect observable implementations
            panic!();
        }
    }

    fn cached_content(self) -> Option<Result<T, E>> {
        match self {
            ObserverState::Changing(None) => None,
            ObserverState::Changing(Some(cache)) => Some(cache),
            ObserverState::Waiting(None, _) => None,
            ObserverState::Waiting(Some(cache), _) => Some(cache),
            ObserverState::Live(content) => Some(content),
        }
    }
}

mod detacher {
    use super::*;

    pub struct ObserverDetacher<R>(Arc<Mutex<Option<R>>>);

    impl<R> ObserverDetacher<R> {
        pub fn new<T, E, W, U>(observer: R) -> (ObserverDetacher<R>, ObserverBox<T, E, W, U>)
        where
            R: Observer<T, E, W, U>,
            T: Send + 'static,
            E: Send + 'static,
            W: Send + 'static,
            U: Update<T> + Send + 'static,
        {
            let arc = Arc::new(Mutex::new(Some(observer)));
            (
                ObserverDetacher(arc.clone()),
                Box::new(DetacherProxy(arc, PhantomData)),
            )
        }

        pub fn detach(self) -> R {
            let opt = match Arc::try_unwrap(self.0) {
                Err(arc) => {
                    // normal path: arc is still shared
                    let mut guard = arc.lock().unwrap();
                    std::mem::take(&mut *guard)
                }
                Ok(mutex) => {
                    // exclusive ownership path
                    mutex.into_inner().unwrap()
                }
            };
            // This is the only code path that returns the inner observer from
            // the `DetachableObserver`. Since the `DetachableObserver` is
            // comsumed here, the unwrap ist safe.
            opt.unwrap()
        }
    }

    struct DetacherProxy<R, T, E, W, U>(Arc<Mutex<Option<R>>>, PhantomData<(T, E, W, U)>);

    impl<R, T, E, W, U> Observer<T, E, W, U> for DetacherProxy<R, T, E, W, U>
    where
        R: Observer<T, E, W, U>,
        T: Send + 'static,
        E: Send + 'static,
        W: Send + 'static,
        U: Update<T> + Send + 'static,
    {
        fn set_changing(&mut self, clear_cache: bool) {
            let mut guard = self.0.lock().unwrap();
            if let Some(observer) = &mut *guard {
                observer.set_changing(clear_cache)
            }
        }

        fn set_waiting(&mut self, clear_cache: bool, marker: W) {
            let mut guard = self.0.lock().unwrap();
            if let Some(observer) = &mut *guard {
                observer.set_waiting(clear_cache, marker)
            }
        }

        fn set_live(
            &mut self,
            content: Option<Result<T, E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            let mut guard = self.0.lock().unwrap();
            match &mut *guard {
                Some(observer) => observer.set_live(content),
                None => Box::new(std::future::pending()),
            }
        }

        fn update(&mut self, update: U) -> Box<dyn Future<Output = ()> + Sync> {
            let mut guard = self.0.lock().unwrap();
            match &mut *guard {
                Some(observer) => observer.update(update),
                None => Box::new(std::future::pending()),
            }
        }
    }
}

mod eval {
    use super::*;

    pub struct Eval<O>(pub O);

    impl<O> Observable for Eval<O>
    where
        O: Observable,
    {
        type T = O::T;
        type E = O::E;
        type W = O::W;
        type U = !;
        fn attach_return<P>(self, observer: P) -> Attached<Self>
        where
            P: Observer<Self::T, Self::E, Self::W, !> + 'static,
        {
            let attached = O::U::attach_eval_return(self.0, observer);
            attached.map(|observable| Eval(observable))
        }

        fn attach_return_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        ) -> Attached<ObservableBox<Self::T, Self::E, Self::W, !>> {
            self.attach_return(observer).map(ObservableBox::new)
        }

        fn attach<P>(self, observer: P) -> Attached<()>
        where
            P: Observer<Self::T, Self::E, Self::W, !> + 'static,
        {
            O::U::attach_eval(self.0, observer)
        }
    }

    pub struct EvalObserver<O, R>
    where
        O: Observable,
        R: Observer<O::T, O::E, O::W, !>,
    {
        pub observer: R,
        cache: Option<O::T>,
    }

    impl<O, R> EvalObserver<O, R>
    where
        O: Observable,
        R: Observer<O::T, O::E, O::W, !>,
    {
        pub fn new(observer: R) -> Self {
            EvalObserver {
                observer,
                cache: None,
            }
        }
    }

    impl<O, R> Observer<O::T, O::E, O::W, O::U> for EvalObserver<O, R>
    where
        O: Observable,
        O::T: Clone,
        R: Observer<O::T, O::E, O::W, !>,
    {
        fn set_changing(&mut self, clear_cache: bool) {
            if clear_cache {
                std::mem::take(&mut self.cache);
            }
            self.observer.set_changing(clear_cache);
        }

        fn set_waiting(&mut self, clear_cache: bool, marker: O::W) {
            if clear_cache {
                std::mem::take(&mut self.cache);
            }
            self.observer.set_waiting(clear_cache, marker);
        }

        fn set_live(
            &mut self,
            content: Option<Result<O::T, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            match &content {
                Some(Ok(value)) => self.cache = Some(value.clone()),
                Some(Err(e)) => self.cache = None,
                None => (),
            }
            self.observer.set_live(content)
        }

        fn update(&mut self, update: O::U) -> Box<dyn Future<Output = ()> + Sync> {
            let cache = match std::mem::take(&mut self.cache) {
                Some(cache) => cache,
                None => panic!(),
            };
            let value = update.apply_update(cache);
            self.cache = Some(value.clone());
            self.observer.set_live(Some(Ok(value)))
        }
    }
}

mod retrieve {
    use super::*;

    struct Retrieve<O>
    where
        O: Observable,
    {
        tx: Option<tokio::sync::oneshot::Sender<Result<O::T, O::E>>>,
    }

    pub fn retrieve<O>(observable: O) -> impl Future<Output = Result<O::T, O::E>> + Send
    where
        O: Observable,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let retrieve_observer = Retrieve::<O> { tx: Some(tx) };
        let attached = observable.eval().attach(retrieve_observer);

        async {
            let result = rx.await;
            drop(attached);

            match result {
                Ok(result) => result,
                Err(_) => {
                    // this should not happen because this task holds a
                    // reference to the observable
                    panic!("observer dropped without receiving a value")
                }
            }
        }
    }

    impl<O> Observer<O::T, O::E, O::W, !> for Retrieve<O>
    where
        O: Observable,
    {
        fn set_changing(&mut self, _clear_cache: bool) {}

        fn set_waiting(&mut self, _clear_cache: bool, _marker: O::W) {}

        fn set_live(
            &mut self,
            content: Option<Result<O::T, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            let content = match content {
                Some(content) => content,
                None => panic!(),
            };

            let tx = std::mem::take(&mut self.tx);
            if let Some(tx) = tx {
                let _ = tx.send(content);
            }
            Box::new(std::future::ready(()))
        }

        fn update(&mut self, update: !) -> Box<dyn Future<Output = ()> + Sync> {
            match update {}
        }
    }
}

mod map {
    use super::*;

    pub struct Map<O, F, A>
    where
        O: Observable,
        F: Fn(O::T) -> A,
    {
        observable: O,
        f: Arc<F>,
    }

    impl<O, A, F> Map<O, F, A>
    where
        O: Observable,
        F: Fn(O::T) -> A,
    {
        pub fn new(observable: O, f: F) -> Map<O, F, A> {
            let f = Arc::new(f);
            Map { observable, f }
        }
    }

    impl<O, F, A> Observable for Map<O, F, A>
    where
        A: Send + Clone + 'static,
        O: Observable,
        F: Fn(O::T) -> A + Send + Sync + 'static,
    {
        type T = A;
        type E = O::E;
        type W = O::W;

        fn attach_return<P: Observer<A, O::E, O::W, !> + 'static>(
            self,
            observer: P,
        ) -> Attached<Self> {
            let f = self.f;
            let attached = self.observable.eval().attach_return(MapObserver {
                f: f.clone(),
                next: observer,
                phantom: PhantomData,
            });
            attached.map(|super::eval::Eval(observable)| Map { observable, f })
        }

        fn attach_return_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        ) -> Attached<ObservableBox<Self::T, Self::E, Self::W, Self::U>> {
            self.attach_return(observer).map(ObservableBox::new)
        }

        fn attach<P: Observer<A, O::E, O::W, !> + 'static>(self, observer: P) -> Attached<()> {
            self.observable.eval().attach(MapObserver {
                f: self.f,
                next: observer,
                phantom: PhantomData,
            })
        }

        fn attach_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        ) -> Attached<()> {
            self.attach(observer)
        }
    }

    pub struct MapObserver<T, E, W, P, F, A>
    where
        P: Observer<A, E, W, !>,
        F: Fn(T) -> A,
    {
        next: P,
        f: Arc<F>,
        phantom: PhantomData<(T, E, W)>,
    }

    impl<T, A, E, W, P, F> Observer<T, E, W, !> for MapObserver<T, E, W, P, F, A>
    where
        P: Observer<A, E, W, !>,
        F: Fn(T) -> A + Send + Sync + 'static,
        A: Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        W: Send + 'static,
    {
        fn set_changing(&mut self, clear_cache: bool) {
            self.next.set_changing(clear_cache)
        }
        fn set_waiting(&mut self, clear_cache: bool, marker: W) {
            self.next.set_waiting(clear_cache, marker)
        }
        fn set_live(
            &mut self,
            content: Option<Result<T, E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            match content {
                Some(Ok(value)) => self.next.set_live(Some(Ok((self.f)(value)))),
                Some(Err(e)) => self.next.set_live(Some(Err(e))),
                None => self.next.set_live(None),
            }
        }

        fn update(&mut self, update: !) -> Box<dyn Future<Output = ()> + Sync> {
            match update {}
        }
    }
}

mod map_items {
    use super::*;

    pub struct MapItems<O, F, A>(pub O, pub F)
    where
        O: Observable,
        O::T: Container,
        O::U: ContainerUpdate<O::T>,
        F: FnMut(<O::T as Container>::I) -> A + Send + 'static,
        A: Send + Clone;

    impl<O, F, A> Observable for MapItems<O, F, A>
    where
        O: Observable,
        O::T: Container,
        O::U: ContainerUpdate<O::T>,
        F: FnMut(<O::T as Container>::I) -> A + Send + 'static,
        A: Send + Clone + 'static,
        <O::T as Container>::C<A>: Send,
        <O::U as ContainerUpdate<O::T>>::U<A>: Send,
    {
        type T = <O::T as Container>::C<A>;
        type E = O::E;
        type W = O::W;
        type U = <O::U as ContainerUpdate<O::T>>::U<A>;

        fn attach_return<P>(self, observer: P) -> Attached<Self>
        where
            P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
        {
            todo!()
        }

        fn attach_return_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        ) -> Attached<ObservableBox<Self::T, Self::E, Self::W, Self::U>> {
            todo!()
        }

        fn attach<P>(self, observer: P) -> Attached<()>
        where
            P: Observer<Self::T, Self::E, Self::W, Self::U> + 'static,
        {
            todo!()
        }

        fn attach_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        ) -> Attached<()> {
            todo!()
        }
    }
}

// struct Subject<T, E> {
//     state: ObserverState<T, E>,
//     observer: Vec<Box<dyn Observer<T, E>>>,
// }
//
// impl<T, E> Observable for &Subject<T, E>
// where
//     T: Send + Copy,
//     E: Send + Copy,
// {
//     type T = T;
//     type E = E;
//     fn attach(self, observer: impl Observer<Self::T, Self::E>) -> AttachedObserver {
//         todo!();
//     }
// }
//
// impl<T, E> Observer<T, E> for &Subject<T, E> {
//     fn set_waiting(&mut self, clear_cache: bool) {}
//     fn set_live(&mut self, content: Option<Result<T, E>>) -> Box<dyn Future<Output = ()> + Sync> {
//         todo!()
//     }
// }

mod share {
    use super::*;

    pub struct Share<O>
    where
        O: Observable,
    {
        state: Mutex<State<O>>,
    }

    enum State<O>
    where
        O: Observable,
    {
        Detached(Option<O>),
        Attached(AttachedState<O>),
    }

    struct AttachedState<O>
    where
        O: Observable,
    {
        attached: Attached<O>,
        next_observer_id: u64,
        // TODO DynObserver / ObserverBox / ObserverObject
        observers: BTreeMap<u64, ObserverBox<O::T, O::E, O::W, O::U>>,
        observer_state: ObserverState<O::T, O::E, O::W>,
    }

    impl<O> Share<O>
    where
        O: Observable,
    {
        pub fn new(observable: O) -> Share<O> {
            Share {
                state: Mutex::new(State::Detached(Some(observable))),
            }
        }
    }

    impl<O> Observable for Arc<Share<O>>
    where
        O: Observable + Send,
        O::T: Clone,
        O::E: Clone,
        O::W: Clone,
    {
        type T = O::T;
        type E = O::E;
        type W = O::W;
        type U = O::U;

        fn attach_return<P: Observer<O::T, O::E, O::W, O::U> + 'static>(
            self,
            observer: P,
        ) -> Attached<Self> {
            let mut observer = observer;
            let mut state = self.state.lock().unwrap();
            let observer_id = match &mut *state {
                State::Detached(observable) => {
                    // attach can only happen once so unwrap is ok
                    let observable = std::mem::take(observable).unwrap();
                    let mut observers: BTreeMap<u64, ObserverBox<O::T, O::E, O::W, O::U>> =
                        BTreeMap::new();
                    observers.insert(0, Box::new(observer));
                    let attached = observable.attach_return(ShareObserver(self.clone()));
                    *state = State::Attached(AttachedState {
                        attached,
                        next_observer_id: 1,
                        observers,
                        observer_state: ObserverState::new(),
                    });
                    0
                }
                State::Attached(attached) => {
                    let observer_id = attached.next_observer_id;
                    attached.next_observer_id += 1;
                    match &attached.observer_state {
                        ObserverState::Changing(None) => (),
                        ObserverState::Changing(Some(_cache)) => {
                            todo!("store cache for subscriber")
                        }
                        ObserverState::Waiting(None, _marker) => {
                            todo!("we need to propagate waiting on subscribe")
                        }
                        ObserverState::Waiting(Some(_cache), _marker) => {
                            todo!("we need to propagate waiting on subscribe + store cache for subscriber")
                        }
                        ObserverState::Live(content) => {
                            let _ = observer.set_live(Some(content.clone()));
                        }
                    }
                    attached.observers.insert(observer_id, Box::new(observer));
                    observer_id
                }
            };
            // release mutex guard to reuse `self`
            drop(state);
            Attached::new(move || {
                let mut state = self.state.lock().unwrap();

                let observer = match &mut *state {
                    State::Detached(_observable) => unreachable!(),
                    State::Attached(attached) => {
                        let observer_box = attached.observers.remove(&observer_id).unwrap();
                        let observer_box = (observer_box as Box<dyn Any>).downcast::<P>().unwrap();
                        let observer = *observer_box;
                        if attached.observers.is_empty() {
                            if let State::Attached(attached) =
                                std::mem::replace(&mut *state, State::Detached(None))
                            {
                                let observable = attached.attached.detach();
                                *state = State::Detached(Some(observable));
                            } else {
                                unreachable!()
                            }
                        }
                        observer
                    }
                };
                drop(state);

                self
            })
        }

        fn attach_return_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        ) -> Attached<ObservableBox<Self::T, Self::E, Self::W, Self::U>> {
            self.attach_return(observer).map(ObservableBox::new)
        }

        fn attach_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        ) -> Attached<()> {
            self.attach(observer)
        }
    }

    impl<O> SharedObservable for Arc<self::share::Share<O>>
    where
        O: Observable + Send,
        O::T: Clone,
        O::E: Clone,
        O::W: Clone,
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

    struct ShareObserver<O>(Arc<Share<O>>)
    where
        O: Observable;

    impl<O> Observer<O::T, O::E, O::W, O::U> for ShareObserver<O>
    where
        O: Observable + Send,
        O::T: Clone,
        O::E: Clone,
        O::W: Clone,
    {
        fn set_changing(&mut self, _clear_cache: bool) {
            let _ = self.0;
            todo!()
        }
        fn set_waiting(&mut self, _clear_cache: bool, _marker: O::W) {
            let _ = self.0;
            todo!()
        }
        fn set_live(
            &mut self,
            _content: Option<Result<O::T, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            todo!()
        }
        fn update(&mut self, _update: O::U) -> Box<dyn Future<Output = ()> + Sync> {
            todo!()
        }
    }
}
