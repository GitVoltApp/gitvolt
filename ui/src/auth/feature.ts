import { registerAppSlot } from '../app/slots';
import { AuthModal } from './AuthModal';

/** The askpass prompt, in the app's `overlay` slot: one for the whole app. */
const off = registerAppSlot('overlay', 'auth', AuthModal);
import.meta.hot?.dispose(off);
