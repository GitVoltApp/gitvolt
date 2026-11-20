import { registerTabSlot } from '../app/slots';
import { Banners } from './Banners';

/** The `banner` slot's notices. */
const off = registerTabSlot('banner', 'banners', Banners);
import.meta.hot?.dispose(off);
