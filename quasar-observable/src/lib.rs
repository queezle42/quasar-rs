#![feature(associated_type_defaults)]
#![feature(never_type)]

use std::boxed::Box;
use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;
use std::mem::{replace, take};
use std::ops::FnMut;
use std::ops::FnOnce;
use std::sync::Arc;
use std::sync::Mutex;

pub mod pure;

mod share;

pub trait Observable: 'static {
    type T: Send;
    type E: Send;
    type W: Send = !;
    type U: Update<Self::T> + Send = !;

    #[must_use]
    fn attach<P>(self, observer: P, selector: Selector) -> AttachedObservable
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U>;

    /// Object-safe variant of `Observable::attach`.
    #[must_use]
    fn attach_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        initial_selector: Selector,
    ) -> AttachedObservable;

    fn share(self) -> impl SharedObservable<T = Self::T, E = Self::E, W = Self::W, U = Self::U>
    where
        Self: Send + Sized,
        Self::T: Clone,
        Self::E: Clone,
        Self::W: Clone,
    {
        share::Share::new(self)
    }

    fn retrieve(self) -> impl Future<Output = Result<Self::T, Self::E>> + Send
    where
        Self: Sized,
    {
        self::retrieve::retrieve(self)
    }

    fn map<F, A>(self, f: F) -> impl Observable<T = A, E = Self::E, W = Self::W, U = !>
    where
        Self: Sized,
        A: Send + Clone + 'static,
        F: Fn(Self::T) -> A + Send + Sync + 'static,
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

    fn attach<P>(self, observer: P, selector: Selector) -> AttachedObservable
    where
        Self: Sized,
        P: Observer<Self::T, Self::E, Self::W, Self::U>,
    {
        self.0.attach_box(Box::new(observer), selector)
    }

    fn attach_box(
        self: Box<Self>,
        observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
        selector: Selector,
    ) -> AttachedObservable {
        self.0.attach_box(observer, selector)
    }
}

impl<T, E, W, U> SharedObservable for SharedObservableBox<T, E, W, U>
where
    T: Send + 'static,
    E: Send + 'static,
    W: Send + 'static,
    U: Update<T> + Send + 'static,
{
    fn clone_box(&self) -> SharedObservableBox<Self::T, Self::E, Self::W, Self::U> {
        self.0.clone_box()
    }
}

#[derive(Clone, PartialEq, Default)]
pub enum Selector {
    #[default]
    Nothing,
    All,
}

impl Selector {
    fn combine(selectors: impl Iterator<Item = Selector>) -> Selector {
        for selector in selectors {
            match selector {
                Selector::All => return Selector::All,
                Selector::Nothing => (),
            }
        }
        Selector::Nothing
    }
}

pub struct AttachedObservable {
    detach: Box<dyn FnOnce() + Send>,
    configure: Box<dyn FnMut(Selector) + Send>,
}

impl Drop for AttachedObservable {
    fn drop(&mut self) {
        let detach = replace(&mut self.detach, Box::new(|| ()));
        detach();
    }
}

impl AttachedObservable {
    pub fn new(
        detach: impl FnOnce() + Send + 'static,
        configure: impl FnMut(Selector) + Send + 'static,
    ) -> Self {
        AttachedObservable {
            detach: Box::new(detach),
            configure: Box::new(configure),
        }
    }

    pub fn detach(self) {
        drop(self)
    }

    pub fn configure(&mut self, selector: Selector) {
        (self.configure)(selector)
    }
}

pub trait Update<T>: EvalUpdate<T> {
    fn apply_update(self, target: T) -> T;
    fn apply_update_mut(self, target: &mut T);
    fn merge(prev: Self, next: Self) -> Self;
    fn merge_update_mut(&mut self, next: Self);
}

impl<T> Update<T> for ! {
    fn apply_update(self, _target: T) -> T {
        match self {}
    }
    fn apply_update_mut(self, _target: &mut T) {
        match self {}
    }
    fn merge(prev: Self, _next: Self) -> Self {
        match prev {}
    }
    fn merge_update_mut(&mut self, _next: Self) {
        match *self {}
    }
}

pub trait EvalUpdate<T> {
    #[must_use]
    fn attach_eval<O, R>(observable: O, observer: R, selector: Selector) -> AttachedObservable
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !>;
}

pub trait EvalUpdateMarker {}

impl<U, T> EvalUpdate<T> for U
where
    U: EvalUpdateMarker,
    T: Send + Clone,
{
    fn attach_eval<O, R>(observable: O, observer: R, selector: Selector) -> AttachedObservable
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !>,
    {
        observable.attach(self::eval::EvalObserver::<O, R>::new(observer), selector)
    }
}

impl<T> EvalUpdate<T> for ! {
    fn attach_eval<O, R>(observable: O, observer: R, selector: Selector) -> AttachedObservable
    where
        O: Observable<T = T, U = Self>,
        R: Observer<O::T, O::E, O::W, !>,
    {
        observable.attach(observer, selector)
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

    fn merge(_prev: Self, _next: Self) -> Self {
        todo!()
    }

    fn merge_update_mut(&mut self, _next: Self) {
        todo!()
    }
}

pub trait Observer<T, E, W, U>: Send + 'static
where
    U: Update<T>,
{
    fn set_changing(&mut self, clear_cache: bool);
    fn set_waiting(&mut self, clear_cache: bool, marker: W);
    fn set_live(&mut self, content: Option<Result<T, E>>) -> Box<dyn Future<Output = ()> + Sync>;
    fn update(&mut self, update: U) -> Box<dyn Future<Output = ()> + Sync>;

    fn done(self: Box<Self>) {}

    fn into_box(self) -> Box<dyn Observer<T, E, W, U>>
    where
        Self: Sized,
    {
        Box::new(self)
    }
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

    fn done(self: Box<Self>) {
        (*self).done()
    }

    fn into_box(self) -> Box<dyn Observer<T, E, W, U>> {
        self
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

mod pending {
    use super::*;

    enum State {
        Blocked,
        Ready,
    }

    pub enum Pending<T, E, U> {
        Unchanged(State),
        Replace(State, Result<T, E>),
        Update(State, U),
    }

    impl<T, E, U> Default for Pending<T, E, U> {
        fn default() -> Self {
            Pending::Unchanged(State::Blocked)
        }
    }

    impl<T, E, U> Pending<T, E, U>
    where
        U: Update<T>,
    {
        pub fn set_blocked(&mut self, clear_cache: bool) {
            if clear_cache {
                match self {
                    Pending::Unchanged(_) => {
                        // already replaced with default by `take`
                    }
                    Pending::Replace(ready, _) => *ready = State::Blocked,
                    Pending::Update(ready, _) => *ready = State::Blocked,
                }
            } else {
                *self = Pending::default();
            }
        }

        pub fn set_live(&mut self, content: Option<Result<T, E>>) {
            if let Some(content) = content {
                *self = Pending::Replace(State::Ready, content)
            } else {
                match self {
                    Pending::Unchanged(ready) => *ready = State::Ready,
                    Pending::Replace(ready, _) => *ready = State::Ready,
                    Pending::Update(ready, _) => *ready = State::Ready,
                }
            }
        }

        pub fn update(&mut self, update: U) {
            *self = match take(self) {
                Pending::Unchanged(_) => panic!(),
                Pending::Replace(_, Ok(result)) => {
                    Pending::Replace(State::Ready, Ok(update.apply_update(result)))
                }
                Pending::Replace(_, Err(_)) => panic!(),
                Pending::Update(_, old_update) => {
                    Pending::Update(State::Ready, U::merge(old_update, update))
                }
            }
        }

        pub fn is_ready(&self) -> bool {
            matches!(
                self,
                Pending::Unchanged(State::Ready)
                    | Pending::Replace(State::Ready, _)
                    | Pending::Update(State::Ready, _)
            )
        }

        pub fn apply_to_observer<P, W>(
            &mut self,
            observer: &mut P,
        ) -> Box<dyn Future<Output = ()> + Sync>
        where
            P: Observer<T, E, W, U>,
        {
            match take(self) {
                Pending::Unchanged(State::Ready) => observer.set_live(None),
                Pending::Replace(State::Ready, result) => observer.set_live(Some(result)),
                Pending::Update(State::Ready, update) => observer.update(update),
                _ => panic!(),
            }
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
                    take(&mut *guard)
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

        fn attach<P>(self, observer: P, selector: Selector) -> AttachedObservable
        where
            P: Observer<Self::T, Self::E, Self::W, !>,
        {
            O::U::attach_eval(self.0, observer, selector)
        }

        fn attach_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
            selector: Selector,
        ) -> AttachedObservable {
            self.attach(observer, selector)
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
                take(&mut self.cache);
            }
            self.observer.set_changing(clear_cache);
        }

        fn set_waiting(&mut self, clear_cache: bool, marker: O::W) {
            if clear_cache {
                take(&mut self.cache);
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
            let cache = match take(&mut self.cache) {
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
        let attached = observable.eval().attach(retrieve_observer, Selector::All);

        async {
            let result = rx.await;

            // explicitly move attached observable into the async block (or it
            // would be dropped on the first `await`)
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

            let tx = take(&mut self.tx);
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

        fn attach<P: Observer<A, O::E, O::W, !>>(
            self,
            observer: P,
            selector: Selector,
        ) -> AttachedObservable {
            self.observable.eval().attach(
                MapObserver {
                    f: self.f,
                    next: observer,
                    phantom: PhantomData,
                },
                selector,
            )
        }

        fn attach_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
            selector: Selector,
        ) -> AttachedObservable {
            self.attach(observer, selector)
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

        fn attach<P>(self, observer: P, selector: Selector) -> AttachedObservable
        where
            P: Observer<Self::T, Self::E, Self::W, Self::U>,
        {
            todo!()
        }

        fn attach_box(
            self: Box<Self>,
            observer: ObserverBox<Self::T, Self::E, Self::W, Self::U>,
            selector: Selector,
        ) -> AttachedObservable {
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
