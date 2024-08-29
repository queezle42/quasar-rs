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

type T<O> = <<O as Observable>::Puc as Puc>::C<<O as Observable>::I>;
type U<O> = <<O as Observable>::Puc as Puc>::Update<<O as Observable>::I>;

pub trait Observable
where
    Self: Sized + 'static,
    T<Self>: Send,
{
    type I: Send;
    type E: Send;
    type W: Send = !;
    type Puc: Puc = ();

    #[must_use]
    fn attach<P>(self, observer: P) -> impl FnOnce() -> (Self, P) + Send
    where
        P: Observer<T<Self>, Self::E, Self::W, U<Self>> + 'static;

    fn retrieve(self) -> impl Future<Output = Result<T<Self>, Self::E>> + Send {
        self::retrieve::retrieve(self)
    }

    fn share(self) -> share::Share<Self> {
        share::Share::new(self)
    }

    fn map<F, A>(self, f: F) -> self::map::Map<Self, F, A>
    where
        F: FnMut(T<Self>) -> A,
    {
        self::map::Map::new(self, f)
    }

    fn map_items<F, A>(self, f: F) -> self::map_items::MapItems<Self, F, A>
    where
        F: FnMut(Self::I) -> A,
    {
        self::map_items::MapItems::new(self, f)
    }
}

pub fn attach_eval_observer<O, R>(observable: O, observer: R) -> impl FnOnce() -> (O, R) + Send
where
    O: Observable,
    T<O>: Send,
    R: Observer<T<O>, O::E, O::W, !> + 'static,
{
    O::Puc::attach_eval_observer(observable, observer)
}

pub trait Puc {
    type C<I>;
    type Update<I>;

    fn apply_update<I>(target: Self::C<I>, update: Self::Update<I>) -> Self::C<I>;
    fn apply_update_mut<I>(target: &mut Self::C<I>, update: Self::Update<I>);
    fn merge_updates<I>(current: Self::Update<I>, next: Self::Update<I>);

    fn map_items<F, I, A>(target: Self::C<I>, f: F) -> Self::C<A>
    where
        F: FnMut(I) -> A;
    fn map_update<F, I, A>(update: Self::Update<I>, f: F) -> Self::Update<A>
    where
        F: FnMut(Self::Update<I>) -> Self::Update<A>;

    fn eval_observable<O>(observable: O) -> impl Observable<Puc = ()>
    where
        O: Observable<Puc = Self>,
        T<O>: Send,
    {
        self::eval::Eval(observable)
    }

    #[must_use]
    fn attach_eval_observer<O, R>(observable: O, observer: R) -> impl FnOnce() -> (O, R) + Send
    where
        O: Observable,
        T<O>: Send,
        R: Observer<T<O>, O::E, O::W, !> + 'static;
}

impl Puc for () {
    type C<I> = I;
    type Update<I> = !;

    fn apply_update<I>(target: I, update: !) -> I {
        match update {}
    }
    fn apply_update_mut<I>(target: &mut I, update: !) {
        match update {}
    }
    fn merge_updates<I>(current: !, next: !) {
        match current {}
    }

    fn map_items<F, I, A>(target: I, f: F) -> A
    where
        F: FnMut(I) -> A,
    {
        let mut f = f;
        f(target)
    }
    fn map_update<F, I, A>(update: !, _f: F) -> Self::Update<A> {
        match update {}
    }

    #[allow(refining_impl_trait)]
    fn eval_observable<O>(observable: O) -> O {
        observable
    }

    fn attach_eval_observer<O, R>(observable: O, observer: R) -> impl FnOnce() -> (O, R) + Send
    where
        O: Observable,
        T<O>: Send,
        R: Observer<T<O>, O::E, O::W, !> + 'static,
    {
        let detach = observable.attach(self::eval::EvalObserver::<O, R>::new(observer));
        || {
            let (observable, eval_observer) = detach();
            (observable, eval_observer.observer)
        }
    }
}

pub trait Observer<T, E, W, U>: Any + Send {
    fn set_changing(&mut self, clear_cache: bool);
    fn set_waiting(&mut self, clear_cache: bool, marker: W);
    fn set_live(&mut self, content: Option<Result<T, E>>) -> Box<dyn Future<Output = ()> + Sync>;
    fn update(&mut self, update: U) -> Box<dyn Future<Output = ()> + Sync>;
}

pub struct AttachedObserver(Option<Box<dyn FnOnce() + Sync + Send>>);
impl Drop for AttachedObserver {
    fn drop(&mut self) {
        let AttachedObserver(state) = self;
        if let Some(detach) = std::mem::take(state) {
            detach();
        }
    }
}

enum ObserverState<I, E, W, P>
where
    P: Puc,
{
    Changing(Option<Result<P::C<I>, E>>),
    Waiting(Option<Result<P::C<I>, E>>, W),
    Live(Result<P::C<I>, E>),
}

impl<I, E, W, P> ObserverState<I, E, W, P>
where
    P: Puc,
{
    fn new() -> ObserverState<I, E, W, P> {
        ObserverState::Changing(None)
    }

    fn set_changing(self, clear_cache: bool) -> ObserverState<I, E, W, P> {
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

    fn set_waiting(self, clear_cache: bool, marker: W) -> ObserverState<I, E, W, P> {
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

    fn set_live(self, content: Option<Result<P::C<I>, E>>) -> ObserverState<I, E, W, P> {
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

    fn update(self, update: <P as Puc>::Update<I>) -> ObserverState<I, E, W, P> {
        if let Some(Ok(value)) = self.cached_content() {
            ObserverState::Live(Ok(P::apply_update(value, update)))
        } else {
            // this can be the result of incorrect observable implementations
            panic!();
        }
    }

    fn cached_content(self) -> Option<Result<P::C<I>, E>> {
        match self {
            ObserverState::Changing(None) => None,
            ObserverState::Changing(Some(cache)) => Some(cache),
            ObserverState::Waiting(None, _) => None,
            ObserverState::Waiting(Some(cache), _) => Some(cache),
            ObserverState::Live(content) => Some(content),
        }
    }
}

mod eval {
    use super::*;

    pub struct Eval<O>(pub O)
    where
        O: Observable,
        T<O>: Send;

    impl<O> Observable for Eval<O>
    where
        O: Observable,
        T<O>: Send,
    {
        type I = T<O>;
        type E = O::E;
        type W = O::W;
        type Puc = ();
        fn attach<P>(self, observer: P) -> impl FnOnce() -> (Self, P) + Send
        where
            P: Observer<T<Self>, Self::E, Self::W, U<Self>> + 'static,
        {
            let Eval(next) = self;
            let detach = next.attach(EvalObserver::<O, P>::new(observer));
            || {
                let (next, eval_observer) = detach();
                (Eval(next), eval_observer.observer)
            }
        }
    }

    pub struct EvalObserver<O, R>
    where
        O: Observable,
        T<O>: Send,
        R: Observer<T<O>, O::E, O::W, !>,
    {
        pub observer: R,
        observer_state: ObserverState<O::I, O::E, O::W, O::Puc>,
    }

    impl<O, R> EvalObserver<O, R>
    where
        O: Observable,
        T<O>: Send,
        R: Observer<T<O>, O::E, O::W, !>,
    {
        pub fn new(observer: R) -> Self {
            EvalObserver {
                observer,
                observer_state: ObserverState::new(),
            }
        }
    }

    impl<O, R> Observer<T<O>, O::E, O::W, U<O>> for EvalObserver<O, R>
    where
        O: Observable,
        T<O>: Send,
        R: Observer<T<O>, O::E, O::W, !>,
    {
        fn set_changing(&mut self, clear_cache: bool) {
            todo!()
        }

        fn set_waiting(&mut self, clear_cache: bool, marker: O::W) {
            todo!()
        }

        fn set_live(
            &mut self,
            content: Option<Result<T<O>, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            todo!()
        }

        fn update(&mut self, update: U<O>) -> Box<dyn Future<Output = ()> + Sync> {
            todo!()
        }
    }
}

mod retrieve {
    use super::*;

    struct Retrieve<O>
    where
        O: Observable,
        T<O>: Send,
    {
        state: Option<ObserverState<O::I, O::E, O::W, O::Puc>>,
        tx: Option<tokio::sync::oneshot::Sender<Result<T<O>, O::E>>>,
    }

    pub fn retrieve<O>(observable: O) -> impl Future<Output = Result<T<O>, O::E>> + Send
    where
        O: Observable,
        T<O>: Send,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let retrieve = Retrieve::<O> {
            state: Some(ObserverState::new()),
            tx: Some(tx),
        };
        let attached = observable.attach(retrieve);

        async {
            let result = rx.await;
            drop(attached);

            match result {
                Ok(result) => result,
                Err(_) => {
                    // this should not happen because this task holds a reference to
                    // the observable
                    panic!("observer dropped without receiving a value")
                }
            }
        }
    }

    impl<O> Observer<T<O>, O::E, O::W, U<O>> for Retrieve<O>
    where
        O: Observable,
        T<O>: Send,
    {
        fn set_changing(&mut self, clear_cache: bool) {
            let opt = std::mem::take(&mut self.state);
            match opt {
                None => (),
                Some(obs) => match obs.set_changing(clear_cache) {
                    ObserverState::Live(_content) => unreachable!(),
                    other => self.state = Some(other),
                },
            }
        }

        fn set_waiting(&mut self, clear_cache: bool, marker: O::W) {
            let opt = std::mem::take(&mut self.state);
            match opt {
                None => (),
                Some(obs) => match obs.set_waiting(clear_cache, marker) {
                    ObserverState::Live(_content) => unreachable!(),
                    other => self.state = Some(other),
                },
            }
        }

        fn set_live(
            &mut self,
            content: Option<Result<T<O>, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            let opt = std::mem::take(&mut self.state);
            match opt {
                None => (),
                Some(obs) => match obs.set_live(content) {
                    ObserverState::Live(content) => {
                        let tx = std::mem::take(&mut self.tx);
                        if let Some(tx) = tx {
                            let _ = tx.send(content);
                        }
                    }
                    other => self.state = Some(other),
                },
            };
            Box::new(std::future::ready(()))
        }

        fn update(&mut self, update: U<O>) -> Box<dyn Future<Output = ()> + Sync> {
            let opt = std::mem::take(&mut self.state);
            match opt {
                None => (),
                Some(obs) => match obs.update(update) {
                    ObserverState::Live(content) => {
                        let tx = std::mem::take(&mut self.tx);
                        if let Some(tx) = tx {
                            let _ = tx.send(content);
                        }
                    }
                    other => self.state = Some(other),
                },
            };
            Box::new(std::future::ready(()))
        }
    }
}

mod map {
    use super::*;

    pub struct Map<O, F, A>
    where
        O: Observable,
        T<O>: Send,
        F: FnMut(T<O>) -> A,
    {
        observable: O,
        f: F,
    }

    impl<O, A, F> Map<O, F, A>
    where
        O: Observable,
        T<O>: Send,
        F: FnMut(T<O>) -> A,
    {
        pub fn new(observable: O, f: F) -> Map<O, F, A> {
            Map { observable, f }
        }
    }

    impl<O, F, A> Observable for Map<O, F, A>
    where
        A: Send + 'static,
        O: Observable,
        T<O>: Send,
        F: FnMut(T<O>) -> A + Send + 'static,
    {
        type I = A;
        type E = O::E;
        type W = O::W;
        fn attach<P: Observer<A, O::E, O::W, !> + 'static>(
            self,
            observer: P,
        ) -> impl FnOnce() -> (Self, P) + Send {
            // attach
            let detach = attach_eval_observer(
                self.observable,
                MapObserver {
                    f: self.f,
                    next: observer,
                    phantom: PhantomData,
                },
            );
            // detach fn
            || {
                let (
                    observable,
                    MapObserver {
                        next,
                        f,
                        phantom: _,
                    },
                ) = detach();
                (Map { observable, f }, next)
            }
        }
    }

    pub struct MapObserver<T, E, W, P, F, A>
    where
        P: Observer<A, E, W, !>,
        F: FnMut(T) -> A,
    {
        next: P,
        f: F,
        phantom: PhantomData<(T, E, W)>,
    }

    impl<T, A, E, W, P, F> Observer<T, E, W, !> for MapObserver<T, E, W, P, F, A>
    where
        P: Observer<A, E, W, !>,
        F: FnMut(T) -> A + Send + 'static,
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

    pub struct MapItems<O, F, A>
    where
        O: Observable,
        T<O>: Send,
        F: FnMut(O::I) -> A,
    {
        observable: O,
        f: F,
    }

    impl<O, A, F> MapItems<O, F, A>
    where
        O: Observable,
        T<O>: Send,
        F: FnMut(O::I) -> A,
    {
        pub fn new(observable: O, f: F) -> MapItems<O, F, A> {
            MapItems { observable, f }
        }
    }

    impl<O, F, A> Observable for MapItems<O, F, A>
    where
        A: Send + 'static,
        O: Observable,
        T<O>: Send,
        <<O as Observable>::Puc as Puc>::C<A>: Send,
        F: FnMut(O::I) -> A + Send + 'static,
    {
        type I = A;
        type E = O::E;
        type W = O::W;
        type Puc = O::Puc;
        fn attach<P: Observer<T<Self>, Self::E, Self::W, U<Self>> + 'static>(
            self,
            observer: P,
        ) -> impl FnOnce() -> (Self, P) + Send {
            || todo!()
        }
    }

    pub struct MapObserver<T, E, W, U, P, F, A>
    where
        P: Observer<A, E, W, U>,
        F: FnMut(T) -> A,
    {
        next: P,
        f: F,
        phantom: PhantomData<(T, E, W, U)>,
    }

    impl<T, E, W, U, P, F, A> Observer<T, E, W, U> for MapObserver<T, E, W, U, P, F, A>
    where
        P: Observer<A, E, W, U>,
        F: FnMut(T) -> A + Send + 'static,
        A: Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        W: Send + 'static,
        U: Send + 'static,
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
        fn update(&mut self, update: U) -> Box<dyn Future<Output = ()> + Sync> {
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
        T<O>: Send,
    {
        state: std::sync::Mutex<State<O>>,
    }

    enum State<O>
    where
        O: Observable,
        T<O>: Send,
    {
        Detached(Option<O>),
        Attached(AttachedState<O>),
    }

    struct AttachedState<O>
    where
        O: Observable,
        T<O>: Send,
    {
        detach_fn: Box<dyn FnOnce() -> (O, ShareObserver<O>) + Send>,
        next_observer_id: u64,
        observers: BTreeMap<u64, Box<dyn Observer<T<O>, O::E, O::W, U<O>>>>,
        observer_state: ObserverState<O::I, O::E, O::W, O::Puc>,
    }

    impl<O> Share<O>
    where
        O: Observable,
        T<O>: Send,
    {
        pub fn new(observable: O) -> Share<O> {
            Share {
                state: std::sync::Mutex::new(State::Detached(Some(observable))),
            }
        }
    }

    impl<O> Observable for Arc<Share<O>>
    where
        O: Observable + Send,
        T<O>: Send + Clone,
        O::E: Clone,
    {
        type I = O::I;
        type E = O::E;
        type W = O::W;
        type Puc = O::Puc;
        fn attach<P: Observer<T<O>, O::E, O::W, U<O>> + 'static>(
            self,
            observer: P,
        ) -> impl FnOnce() -> (Self, P) + Send {
            let mut observer = observer;
            let mut state = self.state.lock().unwrap();
            let observer_id = match &mut *state {
                State::Detached(observable) => {
                    // attach can only happen once so unwrap is ok
                    let observable = std::mem::take(observable).unwrap();
                    let mut observers: BTreeMap<u64, Box<dyn Observer<T<O>, O::E, O::W, U<O>>>> =
                        BTreeMap::new();
                    observers.insert(0, Box::new(observer));
                    let detach_fn = observable.attach(ShareObserver(self.clone()));
                    *state = State::Attached(AttachedState {
                        detach_fn: Box::new(detach_fn),
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
            move || {
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
                                let (observable, _) = (attached.detach_fn)();
                                *state = State::Detached(Some(observable));
                            } else {
                                unreachable!()
                            }
                        }
                        observer
                    }
                };
                drop(state);

                (self, observer)
            }
        }
    }

    struct ShareObserver<O>(Arc<Share<O>>)
    where
        O: Observable,
        T<O>: Send;

    impl<O> Observer<T<O>, O::E, O::W, U<O>> for ShareObserver<O>
    where
        O: Observable + Send,
        T<O>: Send,
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
            _content: Option<Result<T<O>, O::E>>,
        ) -> Box<dyn Future<Output = ()> + Sync> {
            todo!()
        }
        fn update(&mut self, _update: U<O>) -> Box<dyn Future<Output = ()> + Sync> {
            todo!()
        }
    }
}
