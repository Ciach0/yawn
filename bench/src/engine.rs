/// What a scenario needs from a reactive library. Every scenario is written once against this
/// trait and monomorphised per library, so all of them run the same graph and the same writes.
pub trait Engine: 'static {
    type Sig: Copy + 'static;
    type Memo: Copy + 'static;
    /// What the library hands to a computation to read through (the runtime, or nothing).
    type Rd;
    /// What creates nodes while the graph is built.
    type Bld;
    /// The live graph, which also settles pending work after a write.
    type Run;

    /// Builds a graph, runs the initial effects, and returns whatever `build` returns.
    fn start<R: 'static>(build: impl FnOnce(&mut Self::Bld) -> R + 'static) -> (Self::Run, R);
    fn signal(b: &mut Self::Bld, v: u64) -> Self::Sig;
    fn memo(b: &mut Self::Bld, f: impl Fn(&mut Self::Rd) -> u64 + 'static) -> Self::Memo;
    fn effect(b: &mut Self::Bld, f: impl Fn(&mut Self::Rd) + 'static);
    fn sig(rd: &mut Self::Rd, s: Self::Sig) -> u64;
    fn mem(rd: &mut Self::Rd, m: Self::Memo) -> u64;

    /// A write, including everything needed to finish the work it causes.
    fn set(run: &mut Self::Run, s: Self::Sig, v: u64);
    /// `n` writes that are settled together; the library's batching if it has one.
    fn set_many(run: &mut Self::Run, n: usize, f: impl Fn(usize) -> (Self::Sig, u64));
    fn read_sig(run: &mut Self::Run, s: Self::Sig) -> u64;
    fn read_mem(run: &mut Self::Run, m: Self::Memo) -> u64;
}
